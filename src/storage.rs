use super::*;
use axum::{body::Body, http::Request};
use base64::Engine as _;
use futures_util::StreamExt;
use md5::Md5;
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use std::path::PathBuf;
use std::sync::OnceLock;
use tokio::fs;
use tokio::io::{AsyncReadExt, AsyncSeekExt, AsyncWriteExt};
use tokio_util::io::ReaderStream;

fn html_redirect(location: String) -> Response {
    (
        StatusCode::FOUND,
        [
            (header::LOCATION, location),
            (header::CONTENT_TYPE, "text/html; charset=utf-8".to_string()),
        ],
    )
        .into_response()
}

pub(super) fn storage_dir() -> PathBuf {
    env::var_os("RIZZY_STORAGE_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/storage/files"))
}

fn expires_in(minutes: i64) -> String {
    (Utc::now() + chrono::Duration::minutes(minutes))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

fn encoded_filename(filename: &str) -> String {
    const PATH_SEGMENT: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
        .add(b' ')
        .add(b'"')
        .add(b'#')
        .add(b'%')
        .add(b'/')
        .add(b'?')
        .add(b'\\');
    percent_encoding::utf8_percent_encode(filename, PATH_SEGMENT).to_string()
}

fn image_url(p: &Principal, signed_id: &str, filename: &str) -> String {
    format!(
        "/{}/rails/active_storage/blobs/redirect/{signed_id}/{}",
        p.slug,
        encoded_filename(filename)
    )
}

async fn mark_blob_attached(tx: &mut sqlx::Transaction<'_, Db>, blob_id: &str) -> Result<()> {
    let blob = sqlx::query("SELECT content_type,metadata FROM storage_blobs WHERE id=?")
        .bind(blob_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(db_err)?;
    let content_type: String = blob.get("content_type");
    let raw: String = blob.get("metadata");
    let mut metadata: Value = serde_json::from_str(&raw).unwrap_or_else(|_| json!({}));
    let fields = metadata
        .as_object_mut()
        .ok_or_else(|| bad("Invalid blob metadata"))?;
    fields.insert("identified".into(), json!(true));
    // Active Storage's NullAnalyzer runs inline on first attachment. Media
    // analyzers run later and fill dimensions or duration asynchronously.
    if !["image/", "video/", "audio/"]
        .iter()
        .any(|prefix| content_type.starts_with(prefix))
    {
        fields.insert("analyzed".into(), json!(true));
    }
    let updated = metadata.to_string();
    if updated != raw {
        sqlx::query("UPDATE storage_blobs SET metadata=? WHERE id=?")
            .bind(updated)
            .bind(blob_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}

async fn record_storage_entry_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    account_id: &str,
    board_id: &str,
    record_type: &str,
    record_id: &str,
    blob_id: Option<&str>,
    delta: i64,
    operation: &str,
    user_id: Option<&str>,
    request_id: Option<&str>,
) -> Result<()> {
    if delta == 0 {
        return Ok(());
    }
    sqlx::query("INSERT INTO storage_entries(id,account_id,blob_id,board_id,created_at,delta,operation,recordable_id,recordable_type,user_id,request_id) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
        .bind(id())
        .bind(account_id)
        .bind(blob_id)
        .bind(board_id)
        .bind(now())
        .bind(delta)
        .bind(operation)
        .bind(record_id)
        .bind(record_type)
        .bind(user_id)
        .bind(request_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    Ok(())
}

async fn record_attachment_change_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    account_id: &str,
    record_type: &str,
    record_id: &str,
    blob_id: &str,
    operation: &str,
    user_id: Option<&str>,
    request_id: Option<&str>,
) -> Result<()> {
    let board_query = match record_type {
        "Board" => "SELECT id FROM boards WHERE id=? AND account_id=?",
        "Card" => "SELECT board_id FROM cards WHERE id=? AND account_id=?",
        "Comment" => {
            "SELECT c.board_id FROM comments m JOIN cards c ON c.id=m.card_id WHERE m.id=? AND c.account_id=?"
        }
        _ => return Ok(()),
    };
    let board_id: String = sqlx::query_scalar(board_query)
        .bind(record_id)
        .bind(account_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(db_err)?;
    if operation == "attach" {
        mark_blob_attached(tx, blob_id).await?;
    }
    let bytes: i64 =
        sqlx::query_scalar("SELECT byte_size FROM storage_blobs WHERE id=? AND account_id=?")
            .bind(blob_id)
            .bind(account_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(db_err)?;
    record_storage_entry_tx(
        tx,
        account_id,
        &board_id,
        record_type,
        record_id,
        Some(blob_id),
        if operation == "detach" { -bytes } else { bytes },
        operation,
        user_id,
        request_id,
    )
    .await
}

pub(super) async fn record_board_transfer_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    mysql: bool,
    account_id: &str,
    card_id: &str,
    old_board_id: &str,
    new_board_id: &str,
    user_id: &str,
    request_id: &str,
) -> Result<()> {
    let sum = if mysql {
        "CAST(SUM(b.byte_size) AS SIGNED)"
    } else {
        "SUM(b.byte_size)"
    };
    let query = format!(
        "SELECT a.record_type,a.record_id,{sum} AS bytes FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.account_id=? AND ((a.record_type='Card' AND a.record_id=?) OR (a.record_type='Comment' AND a.record_id IN (SELECT id FROM comments WHERE card_id=?))) GROUP BY a.record_type,a.record_id"
    );
    let rows = sqlx::query(&query)
        .bind(account_id)
        .bind(card_id)
        .bind(card_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(db_err)?;
    for row in rows {
        let record_type: String = row.get("record_type");
        let record_id: String = row.get("record_id");
        let bytes: i64 = row.get("bytes");
        record_storage_entry_tx(
            tx,
            account_id,
            old_board_id,
            &record_type,
            &record_id,
            None,
            -bytes,
            "transfer_out",
            Some(user_id),
            Some(request_id),
        )
        .await?;
        record_storage_entry_tx(
            tx,
            account_id,
            new_board_id,
            &record_type,
            &record_id,
            None,
            bytes,
            "transfer_in",
            Some(user_id),
            Some(request_id),
        )
        .await?;
    }
    Ok(())
}

async fn materialize_owner(
    app: &App,
    account_id: &str,
    owner_type: &str,
    owner_id: &str,
) -> Result<bool> {
    let mut tx = begin_write(app).await.map_err(db_err)?;
    if owner_type == "Board" {
        let exists: i64 =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM boards WHERE id=? AND account_id=?)")
                .bind(owner_id)
                .bind(account_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(db_err)?;
        if exists == 0 {
            return Ok(false);
        }
    }
    let insert = if app.mysql {
        "INSERT IGNORE INTO storage_totals(id,account_id,owner_id,owner_type,created_at,updated_at) VALUES(?,?,?,?,?,?)"
    } else {
        "INSERT OR IGNORE INTO storage_totals(id,account_id,owner_id,owner_type,created_at,updated_at) VALUES(?,?,?,?,?,?)"
    };
    let stamp = now();
    sqlx::query(insert)
        .bind(id())
        .bind(account_id)
        .bind(owner_id)
        .bind(owner_type)
        .bind(&stamp)
        .bind(&stamp)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    let total_query = if app.mysql {
        "SELECT id,last_entry_id,bytes_stored FROM storage_totals WHERE owner_type=? AND owner_id=? FOR UPDATE"
    } else {
        "SELECT id,last_entry_id,bytes_stored FROM storage_totals WHERE owner_type=? AND owner_id=?"
    };
    let Some(total) = sqlx::query(total_query)
        .bind(owner_type)
        .bind(owner_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?
    else {
        return Ok(false);
    };
    let cursor: Option<String> = total.get("last_entry_id");
    let pending_query = match (owner_type, app.mysql) {
        ("Account", true) => {
            "SELECT MAX(id) AS latest,CAST(COALESCE(SUM(delta),0) AS SIGNED) AS delta FROM storage_entries WHERE account_id=? AND id>?"
        }
        ("Account", false) => {
            "SELECT MAX(id) AS latest,COALESCE(SUM(delta),0) AS delta FROM storage_entries WHERE account_id=? AND id>?"
        }
        (_, true) => {
            "SELECT MAX(id) AS latest,CAST(COALESCE(SUM(delta),0) AS SIGNED) AS delta FROM storage_entries WHERE board_id=? AND id>?"
        }
        (_, false) => {
            "SELECT MAX(id) AS latest,COALESCE(SUM(delta),0) AS delta FROM storage_entries WHERE board_id=? AND id>?"
        }
    };
    let pending = sqlx::query(pending_query)
        .bind(owner_id)
        .bind(cursor.as_deref().unwrap_or(""))
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
    let latest: Option<String> = pending.get("latest");
    if let Some(latest) = latest {
        let delta: i64 = pending.get("delta");
        sqlx::query(
            "UPDATE storage_totals SET bytes_stored=?,last_entry_id=?,updated_at=? WHERE id=?",
        )
        .bind(total.get::<i64, _>("bytes_stored") + delta)
        .bind(latest)
        .bind(now())
        .bind(total.get::<String, _>("id"))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(true)
}

pub(super) async fn materialize_pending(app: &App) -> Result<usize> {
    let owners = sqlx::query(
        "SELECT DISTINCT e.account_id,'Account' AS owner_type,e.account_id AS owner_id FROM storage_entries e LEFT JOIN storage_totals t ON t.owner_type='Account' AND t.owner_id=e.account_id WHERE e.id>COALESCE(t.last_entry_id,'') UNION SELECT DISTINCT e.account_id,'Board' AS owner_type,e.board_id AS owner_id FROM storage_entries e JOIN boards b ON b.id=e.board_id LEFT JOIN storage_totals t ON t.owner_type='Board' AND t.owner_id=e.board_id WHERE e.board_id IS NOT NULL AND e.id>COALESCE(t.last_entry_id,'') LIMIT 100",
    )
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let mut count = 0;
    for owner in owners {
        if materialize_owner(
            app,
            &owner.get::<String, _>("account_id"),
            &owner.get::<String, _>("owner_type"),
            &owner.get::<String, _>("owner_id"),
        )
        .await?
        {
            count += 1;
        }
    }
    Ok(count)
}

pub(super) async fn materialize_worker(app: Arc<App>) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        interval.tick().await;
        if let Err(error) = materialize_pending(&app).await {
            eprintln!("storage materialization failed: {}", error.1);
        }
    }
}

async fn reconcile_owner(app: &App, account_id: &str, board_id: Option<&str>) -> Result<bool> {
    let mut tx = begin_write(app).await.map_err(db_err)?;
    if let Some(board_id) = board_id {
        lock_board_for_write(&mut tx, board_id, app.mysql).await?;
        let exists: i64 =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM boards WHERE id=? AND account_id=?)")
                .bind(board_id)
                .bind(account_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(db_err)?;
        if exists == 0 {
            return Ok(false);
        }
    }
    let cursor_sql = if board_id.is_some() {
        "SELECT MAX(id) FROM storage_entries WHERE board_id=?"
    } else {
        "SELECT MAX(id) FROM storage_entries WHERE account_id=?"
    };
    let owner_id = board_id.unwrap_or(account_id);
    let cursor_before: Option<String> = sqlx::query_scalar(cursor_sql)
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
    let sum = if app.mysql {
        "CAST(COALESCE(SUM(b.byte_size),0) AS SIGNED)"
    } else {
        "COALESCE(SUM(b.byte_size),0)"
    };
    let scope = if board_id.is_some() {
        "((a.record_type='Card' AND c.board_id=?) OR (a.record_type='Comment' AND mc.board_id=?) OR (a.record_type='Board' AND a.record_id=?))"
    } else {
        "((a.record_type='Card' AND c.account_id=?) OR (a.record_type='Comment' AND mc.account_id=?) OR (a.record_type='Board' AND bd.account_id=?))"
    };
    let real_sql = format!(
        "SELECT {sum} FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id LEFT JOIN cards c ON a.record_type='Card' AND c.id=a.record_id LEFT JOIN comments m ON a.record_type='Comment' AND m.id=a.record_id LEFT JOIN cards mc ON mc.id=m.card_id LEFT JOIN boards bd ON a.record_type='Board' AND bd.id=a.record_id WHERE a.account_id=? AND {scope}"
    );
    let real_bytes: i64 = sqlx::query_scalar(&real_sql)
        .bind(account_id)
        .bind(owner_id)
        .bind(owner_id)
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
    let cursor_after: Option<String> = sqlx::query_scalar(cursor_sql)
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
    if cursor_before != cursor_after {
        return Ok(false);
    }
    let ledger_scope = if board_id.is_some() {
        "board_id"
    } else {
        "account_id"
    };
    let ledger_sum = if app.mysql {
        "CAST(COALESCE(SUM(delta),0) AS SIGNED)"
    } else {
        "COALESCE(SUM(delta),0)"
    };
    let ledger_sql = format!("SELECT {ledger_sum} FROM storage_entries WHERE {ledger_scope}=?");
    let ledger_bytes: i64 = sqlx::query_scalar(&ledger_sql)
        .bind(owner_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
    let delta = real_bytes - ledger_bytes;
    if delta != 0 {
        sqlx::query("INSERT INTO storage_entries(id,account_id,board_id,created_at,delta,operation) VALUES(?,?,?,?,?,'reconcile')")
            .bind(id())
            .bind(account_id)
            .bind(board_id)
            .bind(now())
            .bind(delta)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(delta != 0)
}

pub(super) async fn reconcile_all(app: &App) -> Result<usize> {
    let boards = sqlx::query("SELECT account_id,id FROM boards ORDER BY account_id,id")
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let mut changed = 0;
    for board in boards {
        changed += reconcile_owner(
            app,
            &board.get::<String, _>("account_id"),
            Some(&board.get::<String, _>("id")),
        )
        .await? as usize;
    }
    let accounts = sqlx::query("SELECT id FROM accounts ORDER BY id")
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    for account in accounts {
        changed += reconcile_owner(app, &account.get::<String, _>("id"), None).await? as usize;
    }
    Ok(changed)
}

pub(super) async fn reconcile_imported_storage_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    account_id: &str,
    mysql: bool,
) -> Result<()> {
    let sum = if mysql {
        "CAST(SUM(bytes) AS SIGNED)"
    } else {
        "SUM(bytes)"
    };
    let query = format!(
        "SELECT board_id,{sum} AS bytes FROM (SELECT CASE a.record_type WHEN 'Card' THEN c.board_id WHEN 'Comment' THEN mc.board_id WHEN 'Board' THEN bd.id END AS board_id,b.byte_size AS bytes FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id LEFT JOIN cards c ON a.record_type='Card' AND c.id=a.record_id LEFT JOIN comments m ON a.record_type='Comment' AND m.id=a.record_id LEFT JOIN cards mc ON mc.id=m.card_id LEFT JOIN boards bd ON a.record_type='Board' AND bd.id=a.record_id WHERE a.account_id=?) attached WHERE board_id IS NOT NULL GROUP BY board_id"
    );
    let boards = sqlx::query(&query)
        .bind(account_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(db_err)?;
    for board in boards {
        let bytes: i64 = board.get("bytes");
        if bytes != 0 {
            sqlx::query("INSERT INTO storage_entries(id,account_id,board_id,created_at,delta,operation) VALUES(?,?,?,?,?,'reconcile')")
                .bind(id())
                .bind(account_id)
                .bind(board.get::<String, _>("board_id"))
                .bind(now())
                .bind(bytes)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;
        }
    }
    let sum = if mysql {
        "CAST(COALESCE(SUM(delta),0) AS SIGNED)"
    } else {
        "COALESCE(SUM(delta),0)"
    };
    let total = sqlx::query(&format!(
        "SELECT MAX(id) AS last_entry_id,{sum} AS bytes_stored FROM storage_entries WHERE account_id=?"
    ))
    .bind(account_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(db_err)?;
    let stamp = now();
    sqlx::query("INSERT INTO storage_totals(id,account_id,owner_id,owner_type,bytes_stored,last_entry_id,created_at,updated_at) VALUES(?,?,?,'Account',?,?,?,?)")
        .bind(id())
        .bind(account_id)
        .bind(account_id)
        .bind(total.get::<i64, _>("bytes_stored"))
        .bind(total.get::<Option<String>, _>("last_entry_id"))
        .bind(&stamp)
        .bind(&stamp)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    Ok(())
}

pub(super) fn public_image_path(slug: i64, signed_id: &str, filename: &str) -> String {
    format!(
        "/{slug}/rails/active_storage/blobs/redirect/{signed_id}/{}",
        encoded_filename(filename)
    )
}

pub(super) async fn image_url_for_card(app: &App, p: &Principal, card_id: &str) -> Result<Value> {
    let row = sqlx::query("SELECT b.signed_id,b.filename FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.record_type='Card' AND a.name='image' AND a.record_id=? AND a.account_id=?")
        .bind(card_id).bind(&p.account_id).fetch_optional(&app.db).await.map_err(db_err)?;
    Ok(match row {
        Some(row) => json!(image_url(
            p,
            &row.get::<String, _>("signed_id"),
            &row.get::<String, _>("filename")
        )),
        None => Value::Null,
    })
}

pub(super) async fn public_image_url_for_card(
    app: &App,
    slug: i64,
    account_id: &str,
    card_id: &str,
) -> Result<Option<String>> {
    let row = sqlx::query("SELECT b.signed_id,b.filename FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.record_type='Card' AND a.name='image' AND a.record_id=? AND a.account_id=?")
        .bind(card_id).bind(account_id).fetch_optional(&app.db).await.map_err(db_err)?;
    Ok(row.map(|row| {
        public_image_path(
            slug,
            &row.get::<String, _>("signed_id"),
            &row.get::<String, _>("filename"),
        )
    }))
}

pub(super) async fn image_urls_for_cards(
    app: &App,
    p: &Principal,
    card_ids: &[String],
) -> Result<HashMap<String, Value>> {
    let rows = rows_for_ids(
        app,
        "SELECT a.record_id,b.signed_id,b.filename FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.record_type='Card' AND a.name='image' AND a.record_id IN (",
        card_ids,
    )
    .await?;
    Ok(rows
        .iter()
        .map(|row| {
            (
                row.get::<String, _>("record_id"),
                json!(image_url(
                    p,
                    &row.get::<String, _>("signed_id"),
                    &row.get::<String, _>("filename")
                )),
            )
        })
        .collect())
}

pub(super) async fn embed_owners_for_cards(
    app: &App,
    card_ids: &[String],
) -> Result<HashSet<String>> {
    let rows = rows_for_ids(
        app,
        "SELECT DISTINCT record_id FROM storage_attachments WHERE record_type='Card' AND name IN ('embed','embeds') AND record_id IN (",
        card_ids,
    )
    .await?;
    Ok(rows.into_iter().map(|row| row.get("record_id")).collect())
}

fn human_size(bytes: i64) -> String {
    if bytes < 1024 {
        format!("{bytes} Bytes")
    } else {
        let (value, unit) = if bytes < 1024 * 1024 {
            (bytes as f64 / 1024.0, "KB")
        } else {
            (bytes as f64 / (1024.0 * 1024.0), "MB")
        };
        let decimals = (2.0 - value.log10().floor()).max(0.0) as usize;
        let number = format!("{value:.decimals$}");
        let number = if number.contains('.') {
            number.trim_end_matches('0').trim_end_matches('.')
        } else {
            &number
        };
        format!("{number} {unit}")
    }
}

fn variable_image(content_type: &str) -> bool {
    matches!(
        content_type.to_ascii_lowercase().as_str(),
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    )
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Variation {
    operation: String,
    width: u32,
    height: u32,
    format: Option<String>,
}

fn variation_key(app: &App, operation: &str, width: u32, height: u32) -> String {
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(&Variation {
            operation: operation.to_owned(),
            width,
            height,
            format: None,
        })
        .expect("static variation"),
    );
    let signature = hmac_sha256(&app.qr_secret, &format!("storage_variation:{payload}"));
    format!("{payload}--{signature}")
}

pub(super) fn event_attachment_preview(
    app: &App,
    slug: i64,
    signed_id: &str,
    filename: &str,
    content_type: &str,
    metadata: &str,
) -> String {
    if variable_image(content_type) {
        let variation = variation_key(app, "limit", 800, 600);
        let url = format!(
            "/{slug}/rails/active_storage/representations/redirect/{signed_id}/{variation}/{}",
            encoded_filename(filename)
        );
        let dimensions: Value = serde_json::from_str(metadata).unwrap_or(Value::Null);
        let size = match (dimensions["width"].as_i64(), dimensions["height"].as_i64()) {
            (Some(width), Some(height)) if width > 0 && height > 0 => {
                format!(" width=\"{width}\" height=\"{height}\"")
            }
            _ => String::new(),
        };
        format!(
            "<img class=\"attachment attachment--image\" src=\"{}\"{size} alt=\"\">",
            escape(&url)
        )
    } else {
        let extension = filename
            .rsplit_once('.')
            .map(|(_, value)| value.to_ascii_lowercase())
            .filter(|value| !value.is_empty())
            .unwrap_or_else(|| "unknown".to_owned());
        let class_extension: String = extension
            .chars()
            .filter(|ch| ch.is_ascii_alphanumeric())
            .collect();
        format!(
            "<div class=\"attachment attachment--file attachment--{class_extension}\"><span class=\"attachment__icon\">{}</span></div>",
            escape(&extension)
        )
    }
}

fn verified_variation(app: &App, key: &str) -> Result<Variation> {
    let (payload, signature) = key.rsplit_once("--").ok_or_else(missing)?;
    let expected = hmac_sha256(&app.qr_secret, &format!("storage_variation:{payload}"));
    if signature.len() != expected.len()
        || signature
            .bytes()
            .zip(expected.bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            != 0
    {
        return Err(missing());
    }
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(payload)
        .map_err(|_| missing())?;
    let variation: Variation = serde_json::from_slice(&bytes).map_err(|_| missing())?;
    if !matches!(variation.operation.as_str(), "limit" | "fill")
        || variation.width == 0
        || variation.height == 0
        || variation.width > 4096
        || variation.height > 4096
        || !matches!(
            variation.format.as_deref(),
            None | Some("jpeg" | "png" | "gif" | "webp")
        )
    {
        return Err(missing());
    }
    Ok(variation)
}

fn embed_html(app: &App, blob: &DbRow, sgid: &str, slug: i64, in_paragraph: bool) -> String {
    let filename: String = blob.get("filename");
    let content_type: String = blob.get("content_type");
    let size: i64 = blob.get("byte_size");
    let signed_id: String = blob.get("signed_id");
    let path = format!(
        "/{slug}/rails/active_storage/blobs/redirect/{signed_id}/{}",
        encoded_filename(&filename)
    );
    let variation = variation_key(app, "limit", 1024, 768);
    let variant = format!(
        "/{slug}/rails/active_storage/representations/redirect/{signed_id}/{variation}/{}",
        encoded_filename(&filename)
    );
    let safe_name = escape(&filename);
    let safe_type = escape(&content_type);
    let safe_sgid = escape(sgid);
    let safe_path = escape(&path);
    let safe_variant = escape(&variant);
    let extension = filename
        .rsplit('.')
        .next()
        .unwrap_or("file")
        .to_ascii_lowercase();
    let class_extension: String = extension
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .collect();
    let (before_block, after_block) = if in_paragraph {
        ("  </action-text-attachment></p>", "<p>")
    } else {
        ("  ", "</action-text-attachment>")
    };
    if variable_image(&content_type) {
        let metadata: String = blob.get("metadata");
        let metadata: Value = serde_json::from_str(&metadata).unwrap_or(Value::Null);
        let width = metadata["width"].as_i64().unwrap_or(0);
        let height = metadata["height"].as_i64().unwrap_or(0);
        let dimensions = if width > 0 && height > 0 {
            format!(" width=\"{width}\" height=\"{height}\"")
        } else {
            String::new()
        };
        let image_dimensions = if width > 0 && height > 0 {
            format!(" width=\"{width}\" height=\"{height}\"")
        } else {
            String::new()
        };
        format!(
            "<action-text-attachment sgid=\"{safe_sgid}\" content-type=\"{safe_type}\" url=\"{safe_path}\" filename=\"{safe_name}\" filesize=\"{size}\"{dimensions} previewable=\"true\">{before_block}<figure class=\"attachment attachment--preview attachment--{class_extension}\">\n    \n  <a data-lightbox-target=\"image\" data-lightbox-caption-value=\"{safe_name}\" href=\"{safe_variant}\">\n    <img{image_dimensions} src=\"{safe_variant}\">\n</a>\n\n    <figcaption class=\"attachment__caption\">\n        <span class=\"attachment__name\">{safe_name}</span>\n      <span> · </span>\n      <span class=\"attachment__size\">{}</span>\n      <span> · </span>\n      <a class=\"attachment__link\" title=\"Download {safe_name}\" href=\"{safe_path}?disposition=attachment\">\n        <span>Download</span>\n</a>    </figcaption>\n  </figure>{after_block}",
            human_size(size)
        )
    } else {
        format!(
            "<action-text-attachment sgid=\"{safe_sgid}\" content-type=\"{safe_type}\" url=\"{safe_path}\" filename=\"{safe_name}\" filesize=\"{size}\">{before_block}<div class=\"attachment attachment--file attachment--{class_extension}\">\n    \n  <span class=\"attachment__icon\">{class_extension}</span>\n\n\n    <div class=\"attachment__caption\">\n      <div>\n          <strong class=\"attachment__name\">{safe_name}</strong>\n      </div>\n      <div>\n        <span class=\"attachment__size\">{}</span>\n        <span> · </span>\n        <a class=\"attachment__link\" title=\"Download {safe_name}\" href=\"{safe_path}?disposition=attachment\">\n          <span>Download</span>\n</a>      </div>\n    </div>\n  </div>{after_block}",
            human_size(size)
        )
    }
}

fn external_attachment(attributes: &str) -> Option<(String, String)> {
    static ATTRIBUTE: OnceLock<regex::Regex> = OnceLock::new();
    let attribute = ATTRIBUTE.get_or_init(|| {
        regex::Regex::new(r#"(?i)([a-z][a-z-]*)\s*=\s*["']([^"']*)["']"#)
            .expect("attachment attribute pattern")
    });
    let fields: HashMap<String, String> = attribute
        .captures_iter(attributes)
        .map(|capture| (capture[1].to_ascii_lowercase(), capture[2].to_owned()))
        .collect();
    let raw_url = fields.get("url")?;
    let parsed = url::Url::parse(raw_url).ok()?;
    if !matches!(parsed.scheme(), "http" | "https") {
        return None;
    }
    let content_type = fields.get("content-type").map(String::as_str).unwrap_or("");
    let caption = fields
        .get("caption")
        .or_else(|| fields.get("filename"))
        .map(String::as_str)
        .unwrap_or("Attachment");
    let filename = fields.get("filename").map(String::as_str).unwrap_or("");
    let safe_url = escape(raw_url);
    let safe_type = escape(content_type);
    let safe_caption = escape(caption);
    let safe_filename = escape(filename);
    let figure = if content_type.starts_with("video/") {
        format!(
            "<figure class=\"attachment attachment--preview attachment--video\"><video controls=\"controls\"><source src=\"{safe_url}\" type=\"{safe_type}\"></video><figcaption class=\"attachment__caption\">{safe_caption}</figcaption></figure>"
        )
    } else if content_type.starts_with("image/") {
        format!(
            "<figure class=\"attachment attachment--preview\"><img src=\"{safe_url}\" alt=\"{safe_caption}\"><figcaption class=\"attachment__caption\">{safe_caption}</figcaption></figure>"
        )
    } else {
        format!("<a class=\"attachment attachment--file\" href=\"{safe_url}\">{safe_caption}</a>")
    };
    Some((
        format!("[{caption}]"),
        format!(
            "<action-text-attachment content-type=\"{safe_type}\" url=\"{safe_url}\" filename=\"{safe_filename}\" caption=\"{safe_caption}\">{figure}</action-text-attachment>"
        ),
    ))
}

pub(super) async fn render_rich_text(
    app: &App,
    account_id: &str,
    slug: i64,
    raw: &str,
) -> Result<(String, String, bool)> {
    if !raw.contains("action-text-attachment") {
        return Ok((rich_text::plain_text(raw), rich_text::html(raw), false));
    }
    static TAG: OnceLock<regex::Regex> = OnceLock::new();
    static SGID: OnceLock<regex::Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| {
        regex::Regex::new(r"(?is)<action-text-attachment\b([^>]*)>.*?</action-text-attachment>")
            .expect("attachment pattern")
    });
    let sgid = SGID.get_or_init(|| {
        regex::Regex::new(r#"(?i)\bsgid\s*=\s*["']([^"']+)["']"#).expect("sgid pattern")
    });
    let mut plain_input = String::new();
    let mut html_input = String::new();
    let mut replacements = Vec::new();
    let mut offset = 0;
    let mut attached = false;
    for (index, matched) in tag.captures_iter(raw).enumerate() {
        let whole = matched.get(0).expect("whole attachment");
        plain_input.push_str(&raw[offset..whole.start()]);
        html_input.push_str(&raw[offset..whole.start()]);
        let attributes = matched.get(1).map(|value| value.as_str()).unwrap_or("");
        let value = sgid
            .captures(attributes)
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str())
            .unwrap_or("");
        let token = format!("RIZZYEMBEDPLACEHOLDER{index}END");
        if value.is_empty() {
            if let Some((plain, markup)) = external_attachment(attributes) {
                if plain_input.ends_with('\n') {
                    plain_input.pop();
                }
                plain_input.push_str(&plain);
                replacements.push((token.clone(), format!("</p>{markup}<p>"), markup));
                attached = true;
                html_input.push_str(&token);
                offset = whole.end();
                continue;
            }
        }
        let blob = sqlx::query("SELECT * FROM storage_blobs WHERE attachable_sgid=? AND account_id=? AND uploaded_at IS NOT NULL")
            .bind(value).bind(account_id).fetch_optional(&app.db).await.map_err(db_err)?;
        if let Some(blob) = blob {
            let filename: String = blob.get("filename");
            if variable_image(&blob.get::<String, _>("content_type")) {
                for (width, height) in [(800, 600), (1024, 768)] {
                    let key = variation_key(app, "limit", width, height);
                    ensure_variation(app, &blob, &key).await?;
                }
            }
            plain_input.push_str(&format!("[{}]", filename));
            replacements.push((
                token.clone(),
                embed_html(app, &blob, value, slug, true),
                embed_html(app, &blob, value, slug, false),
            ));
            attached = true;
        } else {
            let mut user = sqlx::query("SELECT u.name FROM user_attachables s JOIN users u ON u.id=s.user_id WHERE s.sgid=? AND s.account_id=?")
                .bind(value).bind(account_id).fetch_optional(&app.db).await.map_err(db_err)?;
            if user.is_none() {
                if let Some(user_id) = app
                    .sgid_signer
                    .parse_user(value)
                    .or_else(|| value.strip_prefix("gid://fizzy/User/").map(str::to_owned))
                {
                    user = sqlx::query("SELECT name FROM users WHERE id=? AND account_id=?")
                        .bind(&user_id)
                        .bind(account_id)
                        .fetch_optional(&app.db)
                        .await
                        .map_err(db_err)?;
                }
            }
            if let Some(user) = user {
                let name: String = user.get("name");
                let first_name = name.split_whitespace().next().unwrap_or(&name);
                plain_input.push_str(first_name);
                let markup = format!(
                    "<action-text-attachment sgid=\"{}\" content-type=\"application/vnd.actiontext.mention\"><span class=\"mention\">{}</span></action-text-attachment>",
                    escape(value),
                    escape(first_name)
                );
                replacements.push((token.clone(), markup.clone(), markup));
                attached = true;
            } else {
                plain_input.push('☒');
                let invalid = format!(
                    "<action-text-attachment sgid=\"{}\">☒</action-text-attachment>",
                    escape(value)
                );
                replacements.push((token.clone(), invalid.clone(), invalid));
            }
        }
        html_input.push_str(&token);
        offset = whole.end();
    }
    plain_input.push_str(&raw[offset..]);
    html_input.push_str(&raw[offset..]);
    let mut html = rich_text::html(&html_input);
    for (token, paragraph_html, bare_html) in replacements {
        let before = html
            .split_once(&token)
            .map(|(before, _)| before)
            .unwrap_or("");
        let in_paragraph = before
            .rfind("<p>")
            .is_some_and(|opening| before.rfind("</p>").is_none_or(|closing| opening > closing));
        html = html.replace(
            &token,
            if in_paragraph {
                &paragraph_html
            } else {
                &bare_html
            },
        );
    }
    Ok((rich_text::plain_text(&plain_input), html, attached))
}

pub(super) async fn attach_card_image_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    p: &Principal,
    card_id: &str,
    value: Option<&Value>,
) -> Result<Vec<String>> {
    let Some(value) = value else {
        return Ok(Vec::new());
    };
    let blob_id = if value.is_null() {
        None
    } else {
        let signed_id = value
            .as_str()
            .ok_or_else(|| bad("image must be a signed blob ID"))?;
        let row = sqlx::query(
            "SELECT id,uploaded_at FROM storage_blobs WHERE signed_id=? AND account_id=?",
        )
        .bind(signed_id)
        .bind(&p.account_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_err)?
        .ok_or_else(|| bad("Invalid image"))?;
        if row.get::<Option<String>, _>("uploaded_at").is_none() {
            return Err(bad("Image upload is incomplete"));
        }
        let blob_id: String = row.get("id");
        let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM storage_attachments WHERE blob_id=? AND record_type='Card' AND name='image' AND record_id<>?")
            .bind(&blob_id).bind(card_id).fetch_one(&mut **tx).await.map_err(db_err)?;
        if used > 0 {
            return Err(bad("Image blob is already attached"));
        }
        Some(blob_id)
    };
    let existing: Option<String> = sqlx::query_scalar("SELECT blob_id FROM storage_attachments WHERE record_type='Card' AND name='image' AND record_id=?")
        .bind(card_id).fetch_optional(&mut **tx).await.map_err(db_err)?;
    if existing == blob_id {
        return Ok(Vec::new());
    }
    let keys = purge_card_images_tx(
        tx,
        &p.account_id,
        Some(card_id),
        None,
        Some(&p.user_id),
        Some(&p.request_id),
    )
    .await?;
    if let Some(blob_id) = blob_id {
        sqlx::query("INSERT INTO storage_attachments(id,account_id,blob_id,record_type,record_id,name,created_at) VALUES(?,?,?,'Card',?,'image',?)")
            .bind(id()).bind(&p.account_id).bind(&blob_id).bind(card_id).bind(now())
            .execute(&mut **tx).await.map_err(db_err)?;
        record_attachment_change_tx(
            tx,
            &p.account_id,
            "Card",
            card_id,
            &blob_id,
            "attach",
            Some(&p.user_id),
            Some(&p.request_id),
        )
        .await?;
    }
    Ok(keys)
}

pub(super) struct PendingUpload {
    id: String,
    pub signed_id: String,
    attachable_sgid: String,
    key: Option<String>,
    filename: String,
    content_type: String,
    byte_size: i64,
    checksum: String,
    dimensions: Option<(i64, i64)>,
}

impl PendingUpload {
    pub fn keep(mut self) {
        self.key = None;
    }
}

impl Drop for PendingUpload {
    fn drop(&mut self) {
        if let Some(key) = &self.key {
            let _ = std::fs::remove_file(storage_dir().join(key));
        }
    }
}

pub(super) async fn receive_multipart_image(
    app: &App,
    mut field: axum::extract::multipart::Field<'_>,
) -> Result<PendingUpload> {
    let filename = field
        .file_name()
        .unwrap_or("upload")
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or("upload")
        .to_owned();
    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_owned();
    let key = random_token();
    let blob_id = id();
    let mut pending = PendingUpload {
        signed_id: app.sgid_signer.blob_signed_id(&blob_id),
        attachable_sgid: app.sgid_signer.blob(&blob_id),
        id: blob_id,
        key: Some(key.clone()),
        filename,
        content_type,
        byte_size: 0,
        checksum: String::new(),
        dimensions: None,
    };
    fs::create_dir_all(storage_dir())
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Storage unavailable"))?;
    let mut output = fs::File::create(storage_dir().join(&key))
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Storage unavailable"))?;
    let mut digest = Md5::new();
    while let Some(chunk) = field.chunk().await.map_err(|_| bad("Invalid card image"))? {
        pending.byte_size = pending
            .byte_size
            .checked_add(i64::try_from(chunk.len()).map_err(|_| bad("Upload too large"))?)
            .ok_or_else(|| bad("Upload too large"))?;
        digest.update(&chunk);
        output
            .write_all(&chunk)
            .await
            .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Storage unavailable"))?;
    }
    output
        .flush()
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Storage unavailable"))?;
    pending.checksum = base64::engine::general_purpose::STANDARD.encode(digest.finalize());
    pending.dimensions = if pending.content_type.starts_with("image/") {
        let path = storage_dir().join(&key);
        tokio::task::spawn_blocking(move || {
            image::ImageReader::open(path)
                .ok()?
                .with_guessed_format()
                .ok()?
                .into_dimensions()
                .ok()
        })
        .await
        .ok()
        .flatten()
        .map(|(width, height)| (i64::from(width), i64::from(height)))
    } else {
        None
    };
    Ok(pending)
}

pub(super) async fn store_multipart_image_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    p: &Principal,
    upload: &PendingUpload,
) -> Result<()> {
    let timestamp = now();
    sqlx::query("INSERT INTO storage_blobs(id,account_id,storage_key,filename,content_type,byte_size,checksum,signed_id,attachable_sgid,upload_token_hash,upload_expires_at,uploaded_at,width,height,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(&upload.id).bind(&p.account_id).bind(upload.key.as_deref()).bind(&upload.filename).bind(&upload.content_type)
        .bind(upload.byte_size)
        .bind(&upload.checksum).bind(&upload.signed_id).bind(&upload.attachable_sgid)
        .bind(hash(&random_token())).bind(expires_in(60)).bind(&timestamp)
        .bind(upload.dimensions.map(|(width, _)| width)).bind(upload.dimensions.map(|(_, height)| height))
        .bind(&timestamp).execute(&mut **tx).await.map_err(db_err)?;
    Ok(())
}

pub(super) async fn purge_card_images_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    account_id: &str,
    card_id: Option<&str>,
    board_id: Option<&str>,
    user_id: Option<&str>,
    request_id: Option<&str>,
) -> Result<Vec<String>> {
    let rows = sqlx::query("SELECT a.id AS attachment_id,a.record_id,b.id AS blob_id,b.storage_key FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id JOIN cards c ON c.id=a.record_id WHERE a.record_type='Card' AND a.name='image' AND c.account_id=? AND (? IS NULL OR c.id=?) AND (? IS NULL OR c.board_id=?)")
        .bind(account_id)
        .bind(card_id).bind(card_id)
        .bind(board_id).bind(board_id)
        .fetch_all(&mut **tx).await.map_err(db_err)?;
    let keys = Vec::new();
    for row in rows {
        let attachment_id: String = row.get("attachment_id");
        let blob_id: String = row.get("blob_id");
        let record_id: String = row.get("record_id");
        record_attachment_change_tx(
            tx, account_id, "Card", &record_id, &blob_id, "detach", user_id, request_id,
        )
        .await?;
        sqlx::query("DELETE FROM storage_attachments WHERE id=?")
            .bind(&attachment_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM storage_attachments WHERE blob_id=?")
                .bind(&blob_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(db_err)?;
        if remaining == 0 {
            enqueue_blob_purge_tx(tx, account_id, &blob_id).await?;
        }
    }
    Ok(keys)
}

async fn purge_blob_with_variants_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    blob_id: &str,
) -> Result<Vec<String>> {
    let variants = sqlx::query("SELECT v.id AS variant_id,a.blob_id AS derived_id,b.storage_key AS derived_key FROM storage_variant_records v LEFT JOIN storage_attachments a ON a.record_type='ActiveStorage::VariantRecord' AND a.record_id=v.id AND a.name='image' LEFT JOIN storage_blobs b ON b.id=a.blob_id WHERE v.blob_id=?")
        .bind(blob_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(db_err)?;
    let mut keys = Vec::new();
    for variant in variants {
        let variant_id: String = variant.get("variant_id");
        let derived_id: Option<String> = variant.get("derived_id");
        let derived_key: Option<String> = variant.get("derived_key");
        sqlx::query("DELETE FROM storage_attachments WHERE record_type='ActiveStorage::VariantRecord' AND record_id=?")
            .bind(&variant_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        if let Some(derived_id) = derived_id {
            sqlx::query("DELETE FROM storage_blobs WHERE id=?")
                .bind(derived_id)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;
        }
        if let Some(key) = derived_key {
            keys.push(key);
        }
    }
    let key: String = sqlx::query_scalar("SELECT storage_key FROM storage_blobs WHERE id=?")
        .bind(blob_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(db_err)?;
    sqlx::query("DELETE FROM storage_blobs WHERE id=?")
        .bind(blob_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    keys.push(key);
    Ok(keys)
}

async fn enqueue_blob_purge_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    account_id: &str,
    blob_id: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO storage_purge_jobs(blob_id,account_id,state,created_at) SELECT ?,?,'pending',? WHERE NOT EXISTS(SELECT 1 FROM storage_purge_jobs WHERE blob_id=?)")
        .bind(blob_id)
        .bind(account_id)
        .bind(now())
        .bind(blob_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    Ok(())
}

async fn purge_blob_after_detaching_variants_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    account_id: &str,
    blob_id: &str,
) -> Result<Vec<String>> {
    let variants = sqlx::query("SELECT v.id AS variant_id,a.blob_id AS derived_id FROM storage_variant_records v LEFT JOIN storage_attachments a ON a.record_type='ActiveStorage::VariantRecord' AND a.record_id=v.id AND a.name='image' WHERE v.blob_id=?")
        .bind(blob_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(db_err)?;
    for variant in variants {
        let variant_id: String = variant.get("variant_id");
        let derived_id: Option<String> = variant.get("derived_id");
        sqlx::query("DELETE FROM storage_attachments WHERE record_type='ActiveStorage::VariantRecord' AND record_id=?")
            .bind(&variant_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        sqlx::query("DELETE FROM storage_variant_records WHERE id=?")
            .bind(&variant_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        if let Some(derived_id) = derived_id {
            enqueue_blob_purge_tx(tx, account_id, &derived_id).await?;
        }
    }
    let key: String = sqlx::query_scalar("SELECT storage_key FROM storage_blobs WHERE id=?")
        .bind(blob_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(db_err)?;
    sqlx::query("DELETE FROM storage_blobs WHERE id=?")
        .bind(blob_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    Ok(vec![key])
}

pub(super) async fn sync_embeds_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    p: &Principal,
    record_type: &str,
    record_id: &str,
    content: &str,
) -> Result<Vec<String>> {
    sync_embeds_tx_inner(tx, p, record_type, record_id, content, false).await
}

pub(super) async fn purge_record_embeds_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    p: &Principal,
    record_type: &str,
    record_id: &str,
) -> Result<Vec<String>> {
    sync_embeds_tx_inner(tx, p, record_type, record_id, "", true).await
}

async fn sync_embeds_tx_inner(
    tx: &mut sqlx::Transaction<'_, Db>,
    p: &Principal,
    record_type: &str,
    record_id: &str,
    content: &str,
    force_clear: bool,
) -> Result<Vec<String>> {
    if !matches!(record_type, "Board" | "Card" | "Comment") {
        return Err(bad("Invalid embedded attachment owner"));
    }
    // Fizzy retains the association when Action Text is assigned empty or null.
    if content.is_empty() && !force_clear {
        return Ok(Vec::new());
    }
    let mut wanted = HashSet::new();
    for sgid in rich_text::attachment_sgids(content) {
        let blob: Option<String> = sqlx::query_scalar("SELECT id FROM storage_blobs WHERE attachable_sgid=? AND account_id=? AND uploaded_at IS NOT NULL")
            .bind(sgid).bind(&p.account_id).fetch_optional(&mut **tx).await.map_err(db_err)?;
        if let Some(blob) = blob {
            wanted.insert(blob);
        }
    }
    let existing = sqlx::query("SELECT a.id AS attachment_id,a.blob_id,b.storage_key FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.record_type=? AND a.record_id=? AND a.name IN ('embed','embeds') AND a.account_id=?")
        .bind(record_type).bind(record_id).bind(&p.account_id)
        .fetch_all(&mut **tx).await.map_err(db_err)?;
    let mut current = HashSet::new();
    let mut keys = Vec::new();
    for row in existing {
        let blob_id: String = row.get("blob_id");
        if wanted.contains(&blob_id) {
            current.insert(blob_id);
            continue;
        }
        record_attachment_change_tx(
            tx,
            &p.account_id,
            record_type,
            record_id,
            &blob_id,
            "detach",
            Some(&p.user_id),
            Some(&p.request_id),
        )
        .await?;
        sqlx::query("DELETE FROM storage_attachments WHERE id=?")
            .bind(row.get::<String, _>("attachment_id"))
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM storage_attachments WHERE blob_id=?")
                .bind(&blob_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(db_err)?;
        if remaining == 0 {
            keys.extend(purge_blob_with_variants_tx(tx, &blob_id).await?);
        }
    }
    for blob_id in wanted.difference(&current) {
        sqlx::query("INSERT INTO storage_attachments(id,account_id,blob_id,record_type,record_id,name,created_at) VALUES(?,?,?,?,?,'embed',?)")
            .bind(id()).bind(&p.account_id).bind(blob_id).bind(record_type).bind(record_id).bind(now())
            .execute(&mut **tx).await.map_err(db_err)?;
        record_attachment_change_tx(
            tx,
            &p.account_id,
            record_type,
            record_id,
            blob_id,
            "attach",
            Some(&p.user_id),
            Some(&p.request_id),
        )
        .await?;
    }
    Ok(keys)
}

pub(super) async fn purge_card_storage_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    account_id: &str,
    card_id: Option<&str>,
    board_id: Option<&str>,
    user_id: Option<&str>,
    request_id: Option<&str>,
) -> Result<Vec<String>> {
    let rows = sqlx::query("SELECT a.id AS attachment_id,a.blob_id,a.record_type,a.record_id,a.name AS attachment_name,b.storage_key FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.account_id=? AND ((a.record_type='Card' AND a.record_id IN (SELECT id FROM cards WHERE account_id=? AND (? IS NULL OR id=?) AND (? IS NULL OR board_id=?))) OR (a.record_type='Comment' AND a.record_id IN (SELECT m.id FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.account_id=? AND (? IS NULL OR c.id=?) AND (? IS NULL OR c.board_id=?))) OR (a.record_type='Board' AND a.record_id=? AND ? IS NOT NULL))")
        .bind(account_id).bind(account_id).bind(card_id).bind(card_id).bind(board_id).bind(board_id)
        .bind(account_id).bind(card_id).bind(card_id).bind(board_id).bind(board_id)
        .bind(board_id).bind(board_id)
        .fetch_all(&mut **tx).await.map_err(db_err)?;
    let mut keys = Vec::new();
    for row in rows {
        let blob_id: String = row.get("blob_id");
        record_attachment_change_tx(
            tx,
            account_id,
            &row.get::<String, _>("record_type"),
            &row.get::<String, _>("record_id"),
            &blob_id,
            "detach",
            user_id,
            request_id,
        )
        .await?;
        sqlx::query("DELETE FROM storage_attachments WHERE id=?")
            .bind(row.get::<String, _>("attachment_id"))
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM storage_attachments WHERE blob_id=?")
                .bind(&blob_id)
                .fetch_one(&mut **tx)
                .await
                .map_err(db_err)?;
        if remaining == 0 {
            if row.get::<String, _>("record_type") == "Card"
                && row.get::<String, _>("attachment_name") == "image"
            {
                enqueue_blob_purge_tx(tx, account_id, &blob_id).await?;
            } else {
                keys.extend(purge_blob_with_variants_tx(tx, &blob_id).await?);
            }
        }
    }
    Ok(keys)
}

pub(super) async fn remove_files(keys: Vec<String>) {
    for key in keys {
        if let Err(error) = fs::remove_dir_all(storage_dir().join("variants").join(&key)).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("storage variant cleanup failed for {key}: {error}");
            }
        }
        for path in [
            storage_dir().join(&key),
            storage_dir().join(format!("{key}.thumb")),
            storage_dir().join(format!("{key}.large.jpg")),
            storage_dir().join(format!("{key}.large.png")),
            storage_dir().join(format!("{key}.large.gif")),
            storage_dir().join(format!("{key}.large.webp")),
        ] {
            if let Err(error) = fs::remove_file(path).await {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("storage cleanup failed for {key}: {error}");
                }
            }
        }
    }
}

async fn claim_purge_job(app: &App) -> sqlx::Result<Option<String>> {
    if app.mysql {
        let mut tx = app.db.begin().await?;
        let blob_id: Option<String> = sqlx::query_scalar("SELECT blob_id FROM storage_purge_jobs WHERE state='pending' ORDER BY created_at,blob_id LIMIT 1 FOR UPDATE SKIP LOCKED")
            .fetch_optional(&mut *tx)
            .await?;
        if let Some(blob_id) = blob_id {
            sqlx::query(
                "UPDATE storage_purge_jobs SET state='in_progress',claimed_at=? WHERE blob_id=?",
            )
            .bind(now())
            .bind(&blob_id)
            .execute(&mut *tx)
            .await?;
            tx.commit().await?;
            Ok(Some(blob_id))
        } else {
            tx.commit().await?;
            Ok(None)
        }
    } else {
        sqlx::query_scalar("UPDATE storage_purge_jobs SET state='in_progress',claimed_at=? WHERE blob_id=(SELECT blob_id FROM storage_purge_jobs WHERE state='pending' ORDER BY created_at,blob_id LIMIT 1) AND state='pending' RETURNING blob_id")
            .bind(now())
            .fetch_optional(&app.db)
            .await
    }
}

async fn purge_claimed_blob(app: &App, blob_id: &str) -> Result<()> {
    let mut tx = begin_write(app).await.map_err(db_err)?;
    let blob_query = if app.mysql {
        "SELECT id,account_id FROM storage_blobs WHERE id=? FOR UPDATE"
    } else {
        "SELECT id,account_id FROM storage_blobs WHERE id=?"
    };
    let blob = sqlx::query(blob_query)
        .bind(blob_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;
    let mut keys = Vec::new();
    if let Some(blob) = blob {
        let attached: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM storage_attachments WHERE blob_id=?")
                .bind(blob_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(db_err)?;
        if attached == 0 {
            keys = purge_blob_after_detaching_variants_tx(
                &mut tx,
                &blob.get::<String, _>("account_id"),
                blob_id,
            )
            .await?;
        }
    }
    sqlx::query("DELETE FROM storage_purge_jobs WHERE blob_id=?")
        .bind(blob_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    remove_files(keys).await;
    Ok(())
}

pub(super) async fn purge_pending(app: &App, limit: usize) -> Result<usize> {
    let stale = (Utc::now() - chrono::Duration::minutes(15))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    sqlx::query("UPDATE storage_purge_jobs SET state='pending',claimed_at=NULL WHERE state='in_progress' AND claimed_at<?")
        .bind(stale)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    let mut completed = 0;
    for _ in 0..limit {
        let Some(blob_id) = claim_purge_job(app).await.map_err(db_err)? else {
            break;
        };
        if let Err(error) = purge_claimed_blob(app, &blob_id).await {
            let _ = sqlx::query(
                "UPDATE storage_purge_jobs SET state='pending',claimed_at=NULL WHERE blob_id=?",
            )
            .bind(&blob_id)
            .execute(&app.db)
            .await;
            return Err(error);
        }
        completed += 1;
    }
    Ok(completed)
}

pub(super) async fn purge_worker(app: Arc<App>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    loop {
        interval.tick().await;
        if let Err(error) = purge_pending(&app, 100).await {
            eprintln!("storage purge worker failed: {}", error.1);
        }
    }
}

pub(super) async fn prepare_user_avatar(
    app: &App,
    p: &Principal,
    value: &Value,
) -> Result<Option<String>> {
    if value.is_null() {
        return Ok(None);
    }
    let signed_id = value.as_str().ok_or_else(|| bad("Invalid avatar"))?;
    let blob = sqlx::query("SELECT id,storage_key,content_type,uploaded_at FROM storage_blobs WHERE signed_id=? AND account_id=?")
        .bind(signed_id).bind(&p.account_id).fetch_optional(&app.db).await.map_err(db_err)?
        .ok_or_else(|| bad("Invalid avatar"))?;
    if blob.get::<Option<String>, _>("uploaded_at").is_none() {
        return Err(bad("Avatar upload is incomplete"));
    }
    let content_type: String = blob.get("content_type");
    if !matches!(
        content_type.as_str(),
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    ) {
        return Err(bad("Avatar must be a JPEG, PNG, GIF, or WebP image"));
    }
    let key: String = blob.get("storage_key");
    let path = storage_dir().join(&key);
    let thumb = storage_dir().join(format!("{key}.thumb"));
    let image_format = match content_type.as_str() {
        "image/png" => image::ImageFormat::Png,
        "image/gif" => image::ImageFormat::Gif,
        "image/webp" => image::ImageFormat::WebP,
        _ => image::ImageFormat::Jpeg,
    };
    tokio::task::spawn_blocking(move || -> Result<()> {
        let reader = image::ImageReader::open(&path)
            .map_err(|_| bad("Invalid avatar image"))?
            .with_guessed_format()
            .map_err(|_| bad("Invalid avatar image"))?;
        let (width, height) = reader
            .into_dimensions()
            .map_err(|_| bad("Invalid avatar image"))?;
        if width > 4096 || height > 4096 {
            return Err(bad("Avatar dimensions must be at most 4096px"));
        }
        let picture = image::ImageReader::open(&path)
            .map_err(|_| bad("Invalid avatar image"))?
            .with_guessed_format()
            .map_err(|_| bad("Invalid avatar image"))?
            .decode()
            .map_err(|_| bad("Invalid avatar image"))?;
        if !thumb.exists() {
            let resized = picture.resize_to_fill(256, 256, image::imageops::FilterType::Lanczos3);
            let temporary = thumb.with_extension(format!("thumb.{}.tmp", random_token()));
            resized
                .save_with_format(&temporary, image_format)
                .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Avatar thumbnail failed"))?;
            std::fs::rename(temporary, thumb)
                .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Avatar thumbnail failed"))?;
        }
        Ok(())
    })
    .await
    .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Avatar thumbnail failed"))??;
    Ok(Some(blob.get("id")))
}

pub(super) async fn attach_user_avatar_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    p: &Principal,
    user_id: &str,
    blob_id: Option<&str>,
) -> Result<Vec<String>> {
    let existing: Option<String> = sqlx::query_scalar("SELECT blob_id FROM storage_attachments WHERE record_type='User' AND name='avatar' AND record_id=?")
        .bind(user_id).fetch_optional(&mut **tx).await.map_err(db_err)?;
    if existing.as_deref() == blob_id {
        return Ok(Vec::new());
    }
    if let Some(blob_id) = blob_id {
        let used: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM storage_attachments WHERE blob_id=? AND record_type='User' AND name='avatar' AND record_id<>?")
            .bind(blob_id).bind(user_id).fetch_one(&mut **tx).await.map_err(db_err)?;
        if used > 0 {
            return Err(bad("Avatar blob is already attached"));
        }
    }
    let keys = detach_user_avatar_tx(tx, &p.account_id, user_id).await?;
    if let Some(blob_id) = blob_id {
        sqlx::query("INSERT INTO storage_attachments(id,account_id,blob_id,record_type,record_id,name,created_at) VALUES(?,?,?,'User',?,'avatar',?)")
            .bind(id()).bind(&p.account_id).bind(blob_id).bind(user_id).bind(now())
            .execute(&mut **tx).await.map_err(db_err)?;
        mark_blob_attached(tx, blob_id).await?;
    }
    Ok(keys)
}

pub(super) async fn detach_user_avatar_tx(
    tx: &mut sqlx::Transaction<'_, Db>,
    account_id: &str,
    user_id: &str,
) -> Result<Vec<String>> {
    let row = sqlx::query("SELECT a.id AS attachment_id,b.id AS blob_id FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.record_type='User' AND a.name='avatar' AND a.record_id=? AND a.account_id=?")
        .bind(user_id).bind(account_id).fetch_optional(&mut **tx).await.map_err(db_err)?;
    let Some(row) = row else {
        return Ok(Vec::new());
    };
    let attachment_id: String = row.get("attachment_id");
    let blob_id: String = row.get("blob_id");
    sqlx::query("DELETE FROM storage_attachments WHERE id=?")
        .bind(attachment_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    let remaining: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM storage_attachments WHERE blob_id=?")
            .bind(&blob_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(db_err)?;
    if remaining != 0 {
        return Ok(Vec::new());
    }
    enqueue_blob_purge_tx(tx, account_id, &blob_id).await?;
    Ok(Vec::new())
}

pub(super) async fn direct_upload(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    if headers
        .get("sec-fetch-site")
        .and_then(|value| value.to_str().ok())
        == Some("cross-site")
        && !headers.contains_key(header::AUTHORIZATION)
    {
        return Err(err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "Cross-site upload denied",
        ));
    }
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let v = nested(&body, "blob");
    let filename = required(v, "filename")?;
    let content_type = required(v, "content_type")?;
    let checksum = required(v, "checksum")?;
    let byte_size = v
        .get("byte_size")
        .and_then(Value::as_i64)
        .filter(|size| *size >= 0)
        .ok_or_else(|| bad("byte_size is required"))?;
    if base64::engine::general_purpose::STANDARD
        .decode(&checksum)
        .map_or(true, |bytes| bytes.len() != 16)
    {
        return Err(bad("Invalid checksum"));
    }
    let id = id();
    let key = random_token();
    let signed_id = app.sgid_signer.blob_signed_id(&id);
    let attachable_sgid = app.sgid_signer.blob(&id);
    let upload_token = random_token();
    let timestamp = now();
    sqlx::query("INSERT INTO storage_blobs(id,account_id,storage_key,filename,content_type,byte_size,checksum,signed_id,attachable_sgid,upload_token_hash,upload_expires_at,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(&id).bind(&p.account_id).bind(&key).bind(&filename).bind(&content_type).bind(byte_size)
        .bind(&checksum).bind(&signed_id).bind(&attachable_sgid).bind(hash(&upload_token))
        .bind(expires_in(60)).bind(&timestamp).execute(&app.db).await.map_err(db_err)?;
    let upload_url = format!(
        "{}/{account}/rails/active_storage/disk/{upload_token}",
        base(&headers)
    );
    Ok(Json(json!({
        "id":id,"account_id":p.account_id,"key":key,"filename":filename,
        "content_type":content_type,"byte_size":byte_size,"checksum":checksum,
        "created_at":timestamp,"metadata":{},"service_name":"local",
        "signed_id":signed_id,"attachable_sgid":attachable_sgid,
        "direct_upload":{"url":upload_url,"headers":{"Content-Type":content_type}}
    })))
}

pub(super) async fn upload_file(
    State(app): Shared,
    Path((account, token)): Path<(String, String)>,
    headers: HeaderMap,
    request: Request<Body>,
) -> Result<StatusCode> {
    let account: i64 = account.parse().map_err(|_| missing())?;
    let blob = sqlx::query("SELECT b.* FROM storage_blobs b JOIN accounts a ON a.id=b.account_id WHERE b.upload_token_hash=? AND a.external_account_id=?")
        .bind(hash(&token)).bind(account).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    if blob.get::<String, _>("upload_expires_at") < now() {
        return Err(missing());
    }
    let content_type: String = blob.get("content_type");
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        != Some(content_type.as_str())
    {
        return Err(bad("Content type does not match upload"));
    }
    let key: String = blob.get("storage_key");
    let dir = storage_dir();
    fs::create_dir_all(&dir)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Storage unavailable"))?;
    let path = dir.join(&key);
    let temp = dir.join(format!("{key}.{}.tmp", random_token()));
    let result: Result<()> = async {
        let mut file = fs::File::create(&temp)
            .await
            .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Storage unavailable"))?;
        let mut stream = request.into_body().into_data_stream();
        let mut digest = Md5::new();
        let mut bytes = 0i64;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| err(StatusCode::BAD_REQUEST, "Upload interrupted"))?;
            bytes += i64::try_from(chunk.len()).map_err(|_| bad("Upload too large"))?;
            if bytes > blob.get::<i64, _>("byte_size") {
                return Err(bad("Upload exceeds declared size"));
            }
            digest.update(&chunk);
            file.write_all(&chunk)
                .await
                .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Storage unavailable"))?;
        }
        file.flush()
            .await
            .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Storage unavailable"))?;
        if bytes != blob.get::<i64, _>("byte_size")
            || base64::engine::general_purpose::STANDARD.encode(digest.finalize())
                != blob.get::<String, _>("checksum")
        {
            return Err(bad("Upload checksum or size mismatch"));
        }
        fs::rename(&temp, &path)
            .await
            .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Storage unavailable"))?;
        Ok(())
    }
    .await;
    if result.is_err() {
        let _ = fs::remove_file(&temp).await;
    }
    result?;
    let dimensions = if content_type.starts_with("image/") {
        tokio::task::spawn_blocking(move || {
            image::ImageReader::open(path)
                .ok()?
                .with_guessed_format()
                .ok()?
                .into_dimensions()
                .ok()
        })
        .await
        .ok()
        .flatten()
    } else {
        None
    };
    sqlx::query("UPDATE storage_blobs SET uploaded_at=?,width=?,height=? WHERE id=?")
        .bind(now())
        .bind(dimensions.map(|(width, _)| i64::from(width)))
        .bind(dimensions.map(|(_, height)| i64::from(height)))
        .bind(blob.get::<String, _>("id"))
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn storage_user_id(app: &App, headers: &HeaderMap, account_id: &str) -> Result<String> {
    let mut auth_headers = headers.clone();
    auth_headers.insert("x-rizzy-json-request", HeaderValue::from_static("1"));
    let (identity_id, _) = identity_auth(app, &auth_headers).await?;
    sqlx::query_scalar("SELECT id FROM users WHERE identity_id=? AND account_id=? AND active=1")
        .bind(identity_id)
        .bind(account_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(|| err(StatusCode::FORBIDDEN, "Account access required"))
}

async fn authorized_blob(
    app: &App,
    account: &str,
    signed_id: &str,
    headers: &HeaderMap,
) -> Result<(DbRow, bool)> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let blob = sqlx::query("SELECT b.* FROM storage_blobs b JOIN accounts a ON a.id=b.account_id WHERE b.signed_id=? AND a.external_account_id=?")
        .bind(signed_id).bind(slug).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    if blob.get::<Option<String>, _>("uploaded_at").is_none() {
        return Err(missing());
    }
    let account_id: String = blob.get("account_id");
    let attachments =
        sqlx::query("SELECT record_type,record_id FROM storage_attachments WHERE blob_id=?")
            .bind(blob.get::<String, _>("id"))
            .fetch_all(&app.db)
            .await
            .map_err(db_err)?;
    let mut cards = Vec::new();
    let mut public = false;
    for attachment in &attachments {
        match attachment.get::<String, _>("record_type").as_str() {
            "User" => {
                let exists: i64 =
                    sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE id=? AND account_id=?")
                        .bind(attachment.get::<String, _>("record_id"))
                        .bind(&account_id)
                        .fetch_one(&app.db)
                        .await
                        .map_err(db_err)?;
                public |= exists > 0;
            }
            "Card" | "Comment" => {
                let record_type: String = attachment.get("record_type");
                let query = if record_type == "Card" {
                    "SELECT c.status,c.board_id,b.publication_key FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.id=? AND c.account_id=?"
                } else {
                    "SELECT c.status,c.board_id,b.publication_key FROM comments m JOIN cards c ON c.id=m.card_id JOIN boards b ON b.id=c.board_id WHERE m.id=? AND c.account_id=?"
                };
                if let Some(card) = sqlx::query(query)
                    .bind(attachment.get::<String, _>("record_id"))
                    .bind(&account_id)
                    .fetch_optional(&app.db)
                    .await
                    .map_err(db_err)?
                {
                    public |= card.get::<String, _>("status") == "published"
                        && card.get::<Option<String>, _>("publication_key").is_some();
                    cards.push(card);
                }
            }
            _ => {}
        }
    }
    if !public {
        let user_id = storage_user_id(app, headers, &account_id).await?;
        let mut accessible = attachments.is_empty();
        for card in &cards {
            let access: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM accesses WHERE board_id=? AND user_id=?")
                    .bind(card.get::<String, _>("board_id"))
                    .bind(&user_id)
                    .fetch_one(&app.db)
                    .await
                    .map_err(db_err)?;
            if access > 0 {
                accessible = true;
                break;
            }
        }
        if !accessible {
            return Err(err(StatusCode::FORBIDDEN, "Blob access required"));
        }
    }
    Ok((blob, public))
}

pub(super) async fn show_blob(
    State(app): Shared,
    Path((account, signed_id, _filename)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Result<Response> {
    let (blob, _) = authorized_blob(&app, &account, &signed_id, &headers).await?;
    let variant = if uri.path().contains("/representations/redirect/") {
        if !variable_image(&blob.get::<String, _>("content_type")) {
            return Err(missing());
        }
        "large"
    } else {
        "original"
    };
    let token = random_token();
    let disposition = if query.get("disposition").map(String::as_str) == Some("attachment") {
        "attachment"
    } else {
        "inline"
    };
    sqlx::query("INSERT INTO storage_download_tokens(token_hash,blob_id,disposition,variant,expires_at) VALUES(?,?,?,?,?)")
        .bind(hash(&token))
        .bind(blob.get::<String, _>("id"))
        .bind(disposition)
        .bind(variant)
        .bind(expires_in(5))
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    let filename: String = blob.get("filename");
    let location = format!(
        "{}/{account}/rails/active_storage/disk/{token}/{}",
        base(&headers),
        encoded_filename(&filename)
    );
    Ok(html_redirect(location))
}

async fn ensure_large_variant(key: &str, content_type: &str) -> Result<PathBuf> {
    let (extension, format) = match content_type.to_ascii_lowercase().as_str() {
        "image/png" => ("png", image::ImageFormat::Png),
        "image/gif" => ("gif", image::ImageFormat::Gif),
        "image/webp" => ("webp", image::ImageFormat::WebP),
        _ => ("jpg", image::ImageFormat::Jpeg),
    };
    let source = storage_dir().join(key);
    let target = storage_dir().join(format!("{key}.large.{extension}"));
    if fs::metadata(&target).await.is_ok() {
        return Ok(target);
    }
    let output = target.clone();
    tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
        let picture = image::ImageReader::open(&source)
            .map_err(|error| error.to_string())?
            .with_guessed_format()
            .map_err(|error| error.to_string())?
            .decode()
            .map_err(|error| error.to_string())?;
        let picture = if picture.width() > 1024 || picture.height() > 768 {
            picture.resize(1024, 768, image::imageops::FilterType::Lanczos3)
        } else {
            picture
        };
        let temporary = output.with_extension(format!("{extension}.{}.tmp", random_token()));
        picture
            .save_with_format(&temporary, format)
            .map_err(|error| error.to_string())?;
        std::fs::rename(&temporary, &output).map_err(|error| error.to_string())?;
        Ok(())
    })
    .await
    .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Image preview failed"))?
    .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Image preview failed"))?;
    Ok(target)
}

async fn ensure_variation(app: &App, blob: &DbRow, variation_key: &str) -> Result<PathBuf> {
    let variation = verified_variation(app, variation_key)?;
    let declared: String = blob.get("content_type");
    if !variable_image(&declared) {
        return Err(missing());
    }
    let format = variation
        .format
        .as_deref()
        .unwrap_or(match declared.as_str() {
            "image/png" => "png",
            "image/gif" => "gif",
            "image/webp" => "webp",
            _ => "jpeg",
        });
    let image_format = match format {
        "png" => image::ImageFormat::Png,
        "gif" => image::ImageFormat::Gif,
        "webp" => image::ImageFormat::WebP,
        _ => image::ImageFormat::Jpeg,
    };
    let extension = if format == "jpeg" { "jpg" } else { format }.to_owned();
    let key: String = blob.get("storage_key");
    let signature = variation_key.rsplit_once("--").ok_or_else(missing)?.1;
    let directory = storage_dir().join("variants").join(&key);
    let target = directory.join(format!("{signature}.{extension}"));
    if fs::metadata(&target).await.is_ok() {
        persist_rich_text_variant(app, blob, &variation, &target).await?;
        persist_avatar_variant(app, blob, &variation, &target).await?;
        return Ok(target);
    }
    fs::create_dir_all(&directory)
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Image preview failed"))?;
    let source = storage_dir().join(&key);
    let output = target.clone();
    let persisted_variation = variation.clone();
    tokio::task::spawn_blocking(move || -> std::result::Result<(), String> {
        if matches!(variation.operation.as_str(), "limit" | "fill")
            && matches!(
                image_format,
                image::ImageFormat::Jpeg
                    | image::ImageFormat::Png
                    | image::ImageFormat::Gif
                    | image::ImageFormat::WebP
            )
        {
            let temporary = output.with_extension(format!("{}.tmp.{extension}", random_token()));
            let source_argument = if variation.operation == "limit"
                && matches!(
                    image_format,
                    image::ImageFormat::Gif | image::ImageFormat::WebP
                ) {
                format!("{}[n=-1]", source.display())
            } else {
                source.display().to_string()
            };
            let mut command = std::process::Command::new("vips");
            command
                .arg("thumbnail_image")
                .arg(source_argument)
                .arg(&temporary)
                .arg(variation.width.to_string())
                .arg("--height")
                .arg(variation.height.to_string());
            if variation.operation == "fill" {
                command.arg("--crop").arg("centre");
            } else {
                command.arg("--size").arg("down");
            }
            let result = command.output();
            if matches!(result, Ok(ref result) if result.status.success()) {
                std::fs::rename(&temporary, &output).map_err(|error| error.to_string())?;
                return Ok(());
            }
            let _ = std::fs::remove_file(&temporary);
        }
        let picture = image::ImageReader::open(source)
            .map_err(|error| error.to_string())?
            .with_guessed_format()
            .map_err(|error| error.to_string())?
            .decode()
            .map_err(|error| error.to_string())?;
        let picture = if variation.operation == "fill" {
            picture.resize_to_fill(
                variation.width,
                variation.height,
                image::imageops::FilterType::Lanczos3,
            )
        } else if picture.width() > variation.width || picture.height() > variation.height {
            picture.resize(
                variation.width,
                variation.height,
                image::imageops::FilterType::Lanczos3,
            )
        } else {
            picture
        };
        let temporary = output.with_extension(format!("{extension}.{}.tmp", random_token()));
        picture
            .save_with_format(&temporary, image_format)
            .map_err(|error| error.to_string())?;
        std::fs::rename(temporary, output).map_err(|error| error.to_string())
    })
    .await
    .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Image preview failed"))?
    .map_err(|error| {
        eprintln!("storage variation render failed: {error}");
        err(StatusCode::INTERNAL_SERVER_ERROR, "Image preview failed")
    })?;
    persist_rich_text_variant(app, blob, &persisted_variation, &target).await?;
    persist_avatar_variant(app, blob, &persisted_variation, &target).await?;
    Ok(target)
}

pub(super) async fn process_user_avatar_variant(app: &App, blob_id: &str) -> Result<()> {
    let blob = sqlx::query("SELECT * FROM storage_blobs WHERE id=?")
        .bind(blob_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let key = variation_key(app, "fill", 256, 256);
    ensure_variation(app, &blob, &key).await?;
    Ok(())
}

fn marshal_positive_fixnum(bytes: &mut Vec<u8>, value: u32) {
    bytes.push(b'i');
    if value == 0 {
        bytes.push(0);
    } else if value <= 122 {
        bytes.push((value + 5) as u8);
    } else {
        let little_endian = value.to_le_bytes();
        let count = little_endian
            .iter()
            .rposition(|byte| *byte != 0)
            .unwrap_or(0)
            + 1;
        bytes.push(count as u8);
        bytes.extend_from_slice(&little_endian[..count]);
    }
}

fn marshal_symbol(bytes: &mut Vec<u8>, name: &str) {
    bytes.push(b':');
    bytes.push((name.len() + 5) as u8);
    bytes.extend_from_slice(name.as_bytes());
}

fn marshal_utf8_string(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(&[b'I', b'"']);
    bytes.push((value.len() + 5) as u8);
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(6);
    marshal_symbol(bytes, "E");
    bytes.push(b'T');
}

fn rich_text_variation_digest(format: &str, width: u32, height: u32) -> String {
    // ActiveStorage::Variation#digest hashes Ruby Marshal's ordered Hash:
    // { format:, loader: { n: -1 }, resize_to_limit: [width, height] }.
    let mut bytes = vec![4, 8, b'{', 8];
    marshal_symbol(&mut bytes, "format");
    marshal_utf8_string(&mut bytes, format);
    marshal_symbol(&mut bytes, "loader");
    bytes.extend_from_slice(&[b'{', 6]);
    marshal_symbol(&mut bytes, "n");
    bytes.extend_from_slice(&[b'i', 0xfa]);
    marshal_symbol(&mut bytes, "resize_to_limit");
    bytes.extend_from_slice(&[b'[', 7]);
    marshal_positive_fixnum(&mut bytes, width);
    marshal_positive_fixnum(&mut bytes, height);
    base64::engine::general_purpose::STANDARD.encode(Sha1::digest(bytes))
}

fn avatar_variation_digest(format: &str) -> String {
    // ActiveStorage::Variation#digest for
    // { format: format, resize_to_fill: [256, 256] }.
    let mut bytes = vec![4, 8, b'{', 7];
    marshal_symbol(&mut bytes, "format");
    marshal_utf8_string(&mut bytes, format);
    marshal_symbol(&mut bytes, "resize_to_fill");
    bytes.extend_from_slice(&[b'[', 7]);
    marshal_positive_fixnum(&mut bytes, 256);
    marshal_positive_fixnum(&mut bytes, 256);
    base64::engine::general_purpose::STANDARD.encode(Sha1::digest(bytes))
}

#[cfg(test)]
#[test]
fn avatar_variant_digest_matches_active_storage() {
    assert_eq!(
        avatar_variation_digest("jpg"),
        "QCHtwKaNWtgBrxkcWm+AKwElcC4="
    );
    assert_eq!(
        avatar_variation_digest("png"),
        "Ul0XHozM8YT3OtF7o1bEa1I7itc="
    );
    assert_eq!(
        avatar_variation_digest("gif"),
        "jv+O+AdtkSYeg5C+IHiR/97I5NI="
    );
    assert_eq!(
        avatar_variation_digest("webp"),
        "+6T9y7mPFOY1C9jjOyjmh065U5o="
    );
}

#[cfg(test)]
#[test]
fn rich_text_variant_digest_matches_active_storage() {
    assert_eq!(
        rich_text_variation_digest("jpg", 800, 600),
        "Bfa0r6FVi88e67+f9GyvuiguoXM="
    );
    assert_eq!(
        rich_text_variation_digest("jpg", 1024, 768),
        "LIHwrDKnUYb2o3ViE1GyFf3u2KM="
    );
    assert_eq!(
        rich_text_variation_digest("png", 800, 600),
        "sTXCDOhTAja1egabPbhk57Bmknk="
    );
    assert_eq!(
        rich_text_variation_digest("png", 1024, 768),
        "xYwHDalWgkJKHCx/iFVLItrYKVs="
    );
    assert_eq!(
        rich_text_variation_digest("gif", 800, 600),
        "sgSbEMr4AmMgi7vYqCUl0FYGbUM="
    );
    assert_eq!(
        rich_text_variation_digest("gif", 1024, 768),
        "Gx9RxwCTekwUv/8TWWggIa3dhJw="
    );
    assert_eq!(
        rich_text_variation_digest("webp", 800, 600),
        "nQMzcKoeueeKiUDLkd2iCYMRfqQ="
    );
    assert_eq!(
        rich_text_variation_digest("webp", 1024, 768),
        "Cs6+3rIJB3PFkf2MUJyPdWF7fhM="
    );
}

fn blob_variant_format(filename: &str, content_type: &str) -> String {
    std::path::Path::new(filename)
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .filter(|extension| {
            matches!(
                (content_type, extension.as_str()),
                ("image/jpeg", "jpg" | "jpeg")
                    | ("image/png", "png")
                    | ("image/gif", "gif")
                    | ("image/webp", "webp")
            )
        })
        .unwrap_or_else(|| {
            match content_type {
                "image/png" => "png",
                "image/gif" => "gif",
                "image/webp" => "webp",
                _ => "jpg",
            }
            .to_owned()
        })
}

async fn persist_rich_text_variant(
    app: &App,
    blob: &DbRow,
    variation: &Variation,
    target: &std::path::Path,
) -> Result<()> {
    if variation.operation != "limit"
        || !matches!(
            (variation.width, variation.height),
            (800, 600) | (1024, 768)
        )
        || variation.format.is_some()
    {
        return Ok(());
    }
    let content_type: String = blob.get("content_type");
    if !matches!(
        content_type.as_str(),
        "image/jpeg" | "image/png" | "image/gif" | "image/webp"
    ) {
        return Ok(());
    }
    let filename: String = blob.get("filename");
    let format = blob_variant_format(&filename, &content_type);
    let digest = rich_text_variation_digest(&format, variation.width, variation.height);
    persist_variant_record(app, blob, &digest, target).await
}

async fn persist_avatar_variant(
    app: &App,
    blob: &DbRow,
    variation: &Variation,
    target: &std::path::Path,
) -> Result<()> {
    if variation.operation != "fill"
        || (variation.width, variation.height) != (256, 256)
        || variation.format.is_some()
    {
        return Ok(());
    }
    let blob_id: String = blob.get("id");
    let attached: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM storage_attachments WHERE blob_id=? AND record_type='User' AND name='avatar'")
        .bind(&blob_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    if attached == 0 {
        return Ok(());
    }
    let filename: String = blob.get("filename");
    let content_type: String = blob.get("content_type");
    let digest = avatar_variation_digest(&blob_variant_format(&filename, &content_type));
    persist_variant_record(app, blob, &digest, target).await
}

async fn persist_variant_record(
    app: &App,
    blob: &DbRow,
    digest: &str,
    target: &std::path::Path,
) -> Result<()> {
    let blob_id: String = blob.get("id");
    let account_id: String = blob.get("account_id");
    let filename: String = blob.get("filename");
    let content_type: String = blob.get("content_type");
    let exists: Option<String> = sqlx::query_scalar(
        "SELECT id FROM storage_variant_records WHERE blob_id=? AND variation_digest=?",
    )
    .bind(&blob_id)
    .bind(&digest)
    .fetch_optional(&app.db)
    .await
    .map_err(db_err)?;
    if exists.is_some() {
        return Ok(());
    }
    let data = fs::read(target).await.map_err(|error| {
        eprintln!("storage variation read failed: {error}");
        err(StatusCode::INTERNAL_SERVER_ERROR, "Image preview failed")
    })?;
    let (width, height) = image::image_dimensions(target).map_err(|error| {
        eprintln!("storage variation dimensions failed: {error}");
        err(StatusCode::INTERNAL_SERVER_ERROR, "Image preview failed")
    })?;
    let mut checksum = Md5::new();
    checksum.update(&data);
    let checksum = base64::engine::general_purpose::STANDARD.encode(checksum.finalize());
    let derived_id = id();
    let derived_key = random_token();
    let record_id = id();
    let timestamp = now();
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let variant_insert = if app.mysql {
        "INSERT INTO storage_variant_records(id,account_id,blob_id,variation_digest) VALUES(?,?,?,?) ON DUPLICATE KEY UPDATE id=id"
    } else {
        "INSERT OR IGNORE INTO storage_variant_records(id,account_id,blob_id,variation_digest) VALUES(?,?,?,?)"
    };
    sqlx::query(variant_insert)
        .bind(&record_id)
        .bind(&account_id)
        .bind(&blob_id)
        .bind(&digest)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    let owning_record: String = sqlx::query_scalar(
        "SELECT id FROM storage_variant_records WHERE blob_id=? AND variation_digest=?",
    )
    .bind(&blob_id)
    .bind(&digest)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_err)?;
    if owning_record != record_id {
        return Ok(());
    }
    sqlx::query("INSERT INTO storage_blobs(id,account_id,storage_key,filename,content_type,byte_size,checksum,metadata,signed_id,attachable_sgid,upload_token_hash,upload_expires_at,uploaded_at,width,height,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(&derived_id)
        .bind(&account_id)
        .bind(&derived_key)
        .bind(&filename)
        .bind(&content_type)
        .bind(data.len() as i64)
        .bind(checksum)
        .bind(json!({"identified":true,"width":width,"height":height,"analyzed":true}).to_string())
        .bind(app.sgid_signer.blob_signed_id(&derived_id))
        .bind(app.sgid_signer.blob(&derived_id))
        .bind(hash(&random_token()))
        .bind(expires_in(60))
        .bind(&timestamp)
        .bind(width as i64)
        .bind(height as i64)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("INSERT INTO storage_attachments(id,account_id,blob_id,record_type,record_id,name,created_at) VALUES(?,?,?,'ActiveStorage::VariantRecord',?,'image',?)")
        .bind(id())
        .bind(&account_id)
        .bind(&derived_id)
        .bind(&record_id)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    let derived_path = storage_dir().join(&derived_key);
    if let Err(error) = fs::write(&derived_path, data).await {
        let _ = fs::remove_file(&derived_path).await;
        eprintln!("storage variation write failed: {error}");
        return Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Image preview failed",
        ));
    }
    if let Err(error) = tx.commit().await {
        let _ = fs::remove_file(&derived_path).await;
        return Err(db_err(error));
    }
    Ok(())
}

pub(super) async fn show_representation(
    State(app): Shared,
    Path((account, signed_id, variation_key, _filename)): Path<(String, String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response> {
    let (blob, _) = authorized_blob(&app, &account, &signed_id, &headers).await?;
    ensure_variation(&app, &blob, &variation_key).await?;
    let token = random_token();
    let disposition = if query.get("disposition").map(String::as_str) == Some("attachment") {
        "attachment"
    } else {
        "inline"
    };
    sqlx::query("INSERT INTO storage_download_tokens(token_hash,blob_id,disposition,variant,expires_at) VALUES(?,?,?,?,?)")
        .bind(hash(&token))
        .bind(blob.get::<String, _>("id"))
        .bind(disposition)
        .bind(format!("v:{variation_key}"))
        .bind(expires_in(5))
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    let filename: String = blob.get("filename");
    let location = format!(
        "{}/{account}/rails/active_storage/disk/{token}/{}",
        base(&headers),
        encoded_filename(&filename)
    );
    Ok(html_redirect(location))
}

pub(super) async fn proxy_representation(
    State(app): Shared,
    Path((account, signed_id, variation_key, _filename)): Path<(String, String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response> {
    let (blob, public) = authorized_blob(&app, &account, &signed_id, &headers).await?;
    let disposition = if query.get("disposition").map(String::as_str) == Some("attachment") {
        "attachment"
    } else {
        "inline"
    };
    serve_blob_file(
        &app,
        &blob,
        &format!("v:{variation_key}"),
        disposition,
        None,
        if public {
            "max-age=3155695200, public, immutable"
        } else {
            "max-age=3155695200, private, immutable"
        },
    )
    .await
}

pub(super) async fn download_file(
    State(app): Shared,
    Path((account, token, _filename)): Path<(String, String, String)>,
) -> Result<Response> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let blob = sqlx::query("SELECT b.*,t.disposition,t.variant FROM storage_download_tokens t JOIN storage_blobs b ON b.id=t.blob_id JOIN accounts a ON a.id=b.account_id WHERE t.token_hash=? AND a.external_account_id=? AND t.expires_at>=?")
        .bind(hash(&token)).bind(slug).bind(now()).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    let variant: String = blob.get("variant");
    let disposition: String = blob.get("disposition");
    serve_blob_file(
        &app,
        &blob,
        &variant,
        &disposition,
        None,
        "public, max-age=300",
    )
    .await
}

pub(super) async fn proxy_blob(
    State(app): Shared,
    Path((account, signed_id, _filename)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response> {
    let (blob, public) = authorized_blob(&app, &account, &signed_id, &headers).await?;
    let disposition = if query.get("disposition").map(String::as_str) == Some("attachment") {
        "attachment"
    } else {
        "inline"
    };
    let mut response = serve_blob_file(
        &app,
        &blob,
        "original",
        disposition,
        headers
            .get(header::RANGE)
            .and_then(|value| value.to_str().ok()),
        if public {
            "max-age=3155695200, public, immutable"
        } else {
            "max-age=3155695200, private, immutable"
        },
    )
    .await?;
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    Ok(response)
}

async fn serve_blob_file(
    app: &App,
    blob: &DbRow,
    variant: &str,
    requested_disposition: &str,
    range_header: Option<&str>,
    cache_control: &str,
) -> Result<Response> {
    let key: String = blob.get("storage_key");
    let declared: String = blob.get("content_type");
    let path = if let Some(variation_key) = variant.strip_prefix("v:") {
        ensure_variation(app, blob, variation_key).await?
    } else if variant == "large" {
        ensure_large_variant(&key, &declared).await?
    } else {
        storage_dir().join(&key)
    };
    let mut file = fs::File::open(&path).await.map_err(|_| missing())?;
    let length = file.metadata().await.map_err(|_| missing())?.len();
    let declared = if let Some(key) = variant.strip_prefix("v:") {
        match verified_variation(app, key)?.format.as_deref() {
            Some("jpeg") => "image/jpeg".to_owned(),
            Some("png") => "image/png".to_owned(),
            Some("gif") => "image/gif".to_owned(),
            Some("webp") => "image/webp".to_owned(),
            _ => declared,
        }
    } else {
        declared
    };
    let lowercase = declared.to_ascii_lowercase();
    let dangerous = !declared.is_ascii()
        || lowercase.starts_with("text/html")
        || lowercase.starts_with("text/x-html")
        || lowercase.starts_with("text/vnd.turbo-stream.html")
        || lowercase.starts_with("application/xhtml+xml")
        || lowercase.starts_with("image/svg+xml")
        || lowercase.starts_with("application/xml")
        || lowercase.starts_with("text/xml");
    let content_type = if dangerous {
        "application/octet-stream"
    } else {
        declared.as_str()
    };
    let range = if let Some(value) = range_header {
        let Some(spec) = value.strip_prefix("bytes=") else {
            return Ok(StatusCode::RANGE_NOT_SATISFIABLE.into_response());
        };
        if spec.contains(',') || length == 0 {
            return Ok(StatusCode::RANGE_NOT_SATISFIABLE.into_response());
        }
        let Some((start, end)) = spec.split_once('-') else {
            return Ok(StatusCode::RANGE_NOT_SATISFIABLE.into_response());
        };
        let range = if start.is_empty() {
            let suffix = end.parse::<u64>().ok().filter(|size| *size > 0);
            suffix.map(|size| (length.saturating_sub(size), length - 1))
        } else {
            let start = start.parse::<u64>().ok().filter(|start| *start < length);
            let end = if end.is_empty() {
                Some(length - 1)
            } else {
                end.parse::<u64>().ok().map(|end| end.min(length - 1))
            };
            start.zip(end).filter(|(start, end)| start <= end)
        };
        let Some(range) = range else {
            return Ok(StatusCode::RANGE_NOT_SATISFIABLE.into_response());
        };
        if range.1 - range.0 + 1 > 5 * 1024 * 1024 {
            return Ok(StatusCode::RANGE_NOT_SATISFIABLE.into_response());
        }
        Some(range)
    } else {
        None
    };
    let mut response = if let Some((start, end)) = range {
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(|_| missing())?;
        let mut bytes =
            vec![
                0;
                usize::try_from(end - start + 1)
                    .map_err(|_| err(StatusCode::RANGE_NOT_SATISFIABLE, "Range too large"))?
            ];
        file.read_exact(&mut bytes).await.map_err(|_| missing())?;
        let mut response = (StatusCode::PARTIAL_CONTENT, bytes).into_response();
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{length}"))
                .expect("valid byte range"),
        );
        response
    } else {
        Body::from_stream(ReaderStream::new(file)).into_response()
    };
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_str(cache_control).expect("static cache control"),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(
            &range
                .map_or(length, |(start, end)| end - start + 1)
                .to_string(),
        )
        .expect("nonnegative blob size"),
    );
    let filename: String = blob.get("filename");
    let safe_filename: String = filename
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '_'
            }
        })
        .collect();
    let disposition = format!(
        "{}; filename=\"{}\"",
        if dangerous || requested_disposition == "attachment" {
            "attachment"
        } else {
            "inline"
        },
        safe_filename
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&disposition).expect("sanitized filename"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

const AVATAR_COLORS: [&str; 18] = [
    "#AF2E1B", "#CC6324", "#3B4B59", "#BFA07A", "#ED8008", "#ED3F1C", "#BF1B1B", "#736B1E",
    "#D07B53", "#736356", "#AD1D1D", "#BF7C2A", "#C09C6F", "#698F9C", "#7C956B", "#5D618F",
    "#3B3633", "#67695E",
];

pub(super) async fn show_avatar(
    State(app): Shared,
    Path((account, user_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let user = sqlx::query("SELECT u.* FROM users u JOIN accounts a ON a.id=u.account_id WHERE u.id=? AND a.external_account_id=?")
        .bind(&user_id).bind(slug).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    if user.get::<String, _>("role") == "system" {
        return Ok((
            StatusCode::FOUND,
            [(header::LOCATION, "/assets/system_user.png")],
        )
            .into_response());
    }
    let avatar = sqlx::query("SELECT b.signed_id,b.filename FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.record_type='User' AND a.name='avatar' AND a.record_id=? AND a.account_id=?")
        .bind(&user_id).bind(user.get::<String, _>("account_id"))
        .fetch_optional(&app.db).await.map_err(db_err)?;
    if let Some(avatar) = avatar {
        let signed_id: String = avatar.get("signed_id");
        let filename: String = avatar.get("filename");
        let variation = variation_key(&app, "fill", 256, 256);
        let location = format!(
            "{}/{account}/rails/active_storage/representations/redirect/{signed_id}/{variation}/{}?disposition=inline",
            base(&headers),
            encoded_filename(&filename)
        );
        return Ok(html_redirect(location));
    }
    let name: String = user.get("name");
    let initials: String = name
        .split_whitespace()
        .filter_map(|part| part.chars().find(|ch| ch.is_alphabetic()))
        .flat_map(char::to_uppercase)
        .collect();
    let color = AVATAR_COLORS[crc32fast::hash(user_id.as_bytes()) as usize % AVATAR_COLORS.len()];
    let width = if initials.chars().count() >= 3 {
        " textLength=\"85%\" lengthAdjust=\"spacingAndGlyphs\""
    } else {
        ""
    };
    let svg = format!(
        "<svg version=\"1.1\" xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 512 512\" class=\"avatar\" aria-hidden=\"true\"><rect width=\"100%\" height=\"100%\" rx=\"50\" fill=\"{color}\"/><text x=\"50%\" y=\"50%\" fill=\"#FFFFFF\" text-anchor=\"middle\" dy=\"0.35em\"{width} font-family=\"-apple-system, BlinkMacSystemFont, Segoe UI, Roboto, Helvetica, Arial, sans-serif\" font-size=\"230\" font-weight=\"800\" letter-spacing=\"-5\">{}</text></svg>",
        escape(&initials)
    );
    let etag = format!(
        "\"{}\"",
        hash(&format!("{user_id}:{name}:{:?}", etag_timezone(&headers)))
    );
    let own_session = if let Some(token) = cookie_value(&headers, "session_token") {
        let session_user: Option<String> = sqlx::query_scalar("SELECT u.id FROM sessions s JOIN users u ON u.identity_id=s.identity_id WHERE s.token_hash=? AND u.account_id=? AND u.active=1")
            .bind(hash(&token)).bind(user.get::<String, _>("account_id"))
            .fetch_optional(&app.db).await.map_err(db_err)?;
        session_user.as_deref() == Some(user_id.as_str())
    } else {
        false
    };
    let cache = if own_session {
        "private, max-age=0"
    } else {
        "max-age=1800, private, stale-while-revalidate=604800"
    };
    if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        == Some(etag.as_str())
    {
        return Ok((
            StatusCode::NOT_MODIFIED,
            [
                (header::ETAG, etag),
                (header::CACHE_CONTROL, cache.to_string()),
            ],
        )
            .into_response());
    }
    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                "image/svg+xml; charset=utf-8".to_string(),
            ),
            (header::ETAG, etag),
            (header::CACHE_CONTROL, cache.to_string()),
        ],
        svg,
    )
        .into_response())
}

pub(super) async fn system_avatar() -> Response {
    (
        [(header::CONTENT_TYPE, "image/png")],
        include_bytes!("../static/system_user.png").as_slice(),
    )
        .into_response()
}

pub(super) async fn show_avatar_representation(
    State(app): Shared,
    Path((account, signed_id, _filename)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let blob = sqlx::query("SELECT b.* FROM storage_blobs b JOIN accounts a ON a.id=b.account_id JOIN storage_attachments x ON x.blob_id=b.id WHERE b.signed_id=? AND a.external_account_id=? AND x.record_type='User' AND x.name='avatar'")
        .bind(signed_id).bind(slug).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    let key: String = blob.get("storage_key");
    if !storage_dir().join(format!("{key}.thumb")).exists() {
        return Err(missing());
    }
    let token = random_token();
    sqlx::query("INSERT INTO storage_download_tokens(token_hash,blob_id,expires_at) VALUES(?,?,?)")
        .bind(hash(&token))
        .bind(blob.get::<String, _>("id"))
        .bind(expires_in(5))
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    let filename: String = blob.get("filename");
    let location = format!(
        "{}/{account}/rails/active_storage/disk/thumb/{token}/{}",
        base(&headers),
        encoded_filename(&filename)
    );
    Ok((StatusCode::FOUND, [(header::LOCATION, location)]).into_response())
}

pub(super) async fn download_avatar_thumbnail(
    State(app): Shared,
    Path((account, token, _filename)): Path<(String, String, String)>,
) -> Result<Response> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let blob = sqlx::query("SELECT b.storage_key,b.content_type FROM storage_download_tokens t JOIN storage_blobs b ON b.id=t.blob_id JOIN accounts a ON a.id=b.account_id JOIN storage_attachments x ON x.blob_id=b.id WHERE t.token_hash=? AND a.external_account_id=? AND t.expires_at>=? AND x.record_type='User' AND x.name='avatar'")
        .bind(hash(&token)).bind(slug).bind(now()).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    let key: String = blob.get("storage_key");
    let file = fs::File::open(storage_dir().join(format!("{key}.thumb")))
        .await
        .map_err(|_| missing())?;
    let length = file.metadata().await.map_err(|_| missing())?.len();
    let mut response = Body::from_stream(ReaderStream::new(file)).into_response();
    let content_type: String = blob.get("content_type");
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&content_type).expect("validated avatar type"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("inline"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string()).expect("file length"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    Ok(response)
}

pub(super) async fn delete_avatar(
    State(app): Shared,
    Path((account, user_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let user = active_user(&app, &p, &user_id).await?;
    can_change_user(&p, &user)?;
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let keys = detach_user_avatar_tx(&mut tx, &p.account_id, &user_id).await?;
    tx.commit().await.map_err(db_err)?;
    remove_files(keys).await;
    if headers.contains_key("x-rizzy-json-request") {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("{}/{account}/users/{user_id}", base(&headers)),
            )],
        )
            .into_response())
    }
}

pub(super) async fn delete_image(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let card = card(&app, &p, &number).await?;
    let card_id: String = card.get("id");
    let (mut tx, _) =
        begin_card_board_write(&app, &card_id, &card.get::<String, _>("board_id")).await?;
    let attached: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM storage_attachments WHERE record_type='Card' AND name='image' AND record_id=?")
        .bind(&card_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
    let keys = purge_card_images_tx(
        &mut tx,
        &p.account_id,
        Some(&card_id),
        None,
        Some(&p.user_id),
        Some(&p.request_id),
    )
    .await?;
    if attached != 0 {
        sqlx::query("UPDATE cards SET updated_at=? WHERE id=?")
            .bind(now())
            .bind(&card_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    remove_files(keys).await;
    if headers.contains_key("x-rizzy-json-request") {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("{}/{account}/cards/{number}", base(&headers)),
            )],
        )
            .into_response())
    }
}
