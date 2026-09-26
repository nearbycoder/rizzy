use super::*;
use axum::body::Body;
use std::io::Write;
use std::path::PathBuf;
use std::sync::OnceLock;
use tokio_util::io::ReaderStream;
use zip::{CompressionMethod, ZipWriter, write::SimpleFileOptions};

fn export_dir() -> PathBuf {
    env::var_os("RIZZY_EXPORT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/exports"))
}

pub(crate) fn export_path(id: &str) -> PathBuf {
    export_dir().join(format!("{id}.zip"))
}

fn json_request(headers: &HeaderMap, name: &str) -> bool {
    name.ends_with(".json")
        || headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("json"))
}

fn iso(value: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(value)
        .map(|date| {
            date.with_timezone(&Utc)
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .unwrap_or_else(|_| value.to_owned())
}

struct ExportBlob {
    sgid: String,
    filename: String,
    content_type: String,
    archive_path: String,
    storage_path: PathBuf,
}

fn archive_filename(filename: &str) -> String {
    filename.replace(['/', '\\'], "_")
}

fn export_rich_text(raw: &str, blobs: &HashMap<String, &ExportBlob>) -> String {
    if raw.is_empty() {
        return String::new();
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
    let mut input = String::new();
    let mut replacements = Vec::new();
    let mut offset = 0;
    for (index, matched) in tag.captures_iter(raw).enumerate() {
        let whole = matched.get(0).expect("whole attachment");
        input.push_str(&raw[offset..whole.start()]);
        let sgid = matched
            .get(1)
            .and_then(|attrs| sgid.captures(attrs.as_str()))
            .and_then(|captures| captures.get(1))
            .map(|value| value.as_str())
            .unwrap_or("");
        let replacement = if let Some(blob) = blobs.get(sgid) {
            let path = escape(&blob.archive_path);
            let filename = escape(&blob.filename);
            if blob.content_type.starts_with("image/") {
                format!("<img src=\"{path}\" alt=\"{filename}\">")
            } else {
                format!("<a href=\"{path}\">{filename}</a>")
            }
        } else {
            "☒".to_owned()
        };
        let token = format!("RIZZYEXPORTATTACHMENT{index}END");
        input.push_str(&token);
        replacements.push((token, replacement));
        offset = whole.end();
    }
    input.push_str(&raw[offset..]);
    let html = rich_text::html(&input);
    let mut inner = html
        .strip_prefix("<div class=\"action-text-content\">\n  ")
        .and_then(|value| value.strip_suffix("\n</div>\n"))
        .unwrap_or(&html)
        .to_owned();
    for (token, replacement) in replacements {
        inner = inner.replace(&token, &replacement);
    }
    inner
}

fn export_view(row: &DbRow, host: &str, slug: i64) -> Value {
    let id: String = row.get("id");
    let mut value = json!({
        "id": id,
        "status": row.get::<String, _>("status"),
        "created_at": row.get::<String, _>("created_at")
    });
    if row.get::<String, _>("status") == "completed" {
        value["download_url"] = json!(format!("{host}/{slug}/exports/{id}/download"));
    }
    value
}

async fn scoped_user(app: &App, p: &Principal, user: &str) -> Result<()> {
    let exists: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND account_id=?)")
            .bind(user)
            .bind(&p.account_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
    if exists == 0 {
        return Err(missing());
    }
    if user != p.user_id {
        return Err(err(
            StatusCode::FORBIDDEN,
            "Cannot access another user's export",
        ));
    }
    Ok(())
}

pub(super) async fn create_user(
    State(app): Shared,
    Path((account, user)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    scoped_user(&app, &p, &user).await?;
    let _guard = app.write_lock.lock().await;
    let cutoff = (Utc::now() - chrono::Duration::hours(24))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM exports WHERE user_id=? AND kind='user' AND created_at>=?",
    )
    .bind(&p.user_id)
    .bind(cutoff)
    .fetch_one(&app.db)
    .await
    .map_err(db_err)?;
    if count >= 10 {
        return Err(err(StatusCode::TOO_MANY_REQUESTS, "Export limit reached"));
    }
    let export_id = id();
    let timestamp = now();
    sqlx::query("INSERT INTO exports(id,account_id,user_id,kind,created_at,updated_at) VALUES(?,?,?,'user',?,?)")
        .bind(&export_id)
        .bind(&p.account_id)
        .bind(&p.user_id)
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    if json_request(&headers, &user) {
        let row = sqlx::query("SELECT * FROM exports WHERE id=?")
            .bind(&export_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
        app.export_notify.notify_one();
        return Ok((
            StatusCode::CREATED,
            Json(export_view(&row, &base(&headers), p.slug)),
        )
            .into_response());
    }
    app.export_notify.notify_one();
    Ok((
        StatusCode::SEE_OTHER,
        [(header::LOCATION, format!("/{account}/users/{user}"))],
    )
        .into_response())
}

pub(super) async fn show_user(
    State(app): Shared,
    Path((account, user, export_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    scoped_user(&app, &p, &user).await?;
    let id = export_id.trim_end_matches(".json");
    let row = sqlx::query(
        "SELECT * FROM exports WHERE id=? AND account_id=? AND user_id=? AND kind='user'",
    )
    .bind(id)
    .bind(&p.account_id)
    .bind(&p.user_id)
    .fetch_optional(&app.db)
    .await
    .map_err(db_err)?;
    if json_request(&headers, &export_id) {
        return Ok(Json(export_view(
            &row.ok_or_else(missing)?,
            &base(&headers),
            p.slug,
        ))
        .into_response());
    }
    let completed = row
        .as_ref()
        .is_some_and(|r| r.get::<String, _>("status") == "completed");
    let html = if completed {
        format!(
            "<!doctype html><html><head><title>Download Export</title></head><body><h2>Download Export</h2><div>Your export is ready.</div><a id=\"download-link\" href=\"/{account}/exports/{id}/download\">Download your data</a></body></html>"
        )
    } else {
        "<!doctype html><html><head><title>Download Expired</title></head><body><h2>Download Expired</h2><div>That download link has expired.</div></body></html>".to_owned()
    };
    Ok(Html(html).into_response())
}

pub(super) async fn create_account(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.account_admin()?;
    p.write()?;
    let _guard = app.write_lock.lock().await;
    let cutoff = (Utc::now() - chrono::Duration::hours(24))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM exports WHERE account_id=? AND kind='account' AND created_at>=?",
    )
    .bind(&p.account_id)
    .bind(cutoff)
    .fetch_one(&app.db)
    .await
    .map_err(db_err)?;
    if count >= 10 {
        return Err(err(StatusCode::TOO_MANY_REQUESTS, "Export limit reached"));
    }
    let export_id = id();
    let timestamp = now();
    sqlx::query("INSERT INTO exports(id,account_id,user_id,kind,created_at,updated_at) VALUES(?,?,?,'account',?,?)")
        .bind(&export_id).bind(&p.account_id).bind(&p.user_id).bind(&timestamp).bind(&timestamp)
        .execute(&app.db).await.map_err(db_err)?;
    if json_request(&headers, &account) {
        let row = sqlx::query("SELECT * FROM exports WHERE id=?")
            .bind(&export_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
        app.export_notify.notify_one();
        return Ok((
            StatusCode::CREATED,
            Json(export_view(&row, &base(&headers), p.slug)),
        )
            .into_response());
    }
    app.export_notify.notify_one();
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, format!("/{account}/account/settings"))],
    )
        .into_response())
}

pub(super) async fn show_account(
    State(app): Shared,
    Path((account, export_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.account_admin()?;
    let id = export_id.trim_end_matches(".json");
    let row = sqlx::query(
        "SELECT * FROM exports WHERE id=? AND account_id=? AND user_id=? AND kind='account'",
    )
    .bind(id)
    .bind(&p.account_id)
    .bind(&p.user_id)
    .fetch_optional(&app.db)
    .await
    .map_err(db_err)?;
    if json_request(&headers, &export_id) {
        return Ok(Json(export_view(
            &row.ok_or_else(missing)?,
            &base(&headers),
            p.slug,
        ))
        .into_response());
    }
    let completed = row
        .as_ref()
        .is_some_and(|r| r.get::<String, _>("status") == "completed");
    let html = if completed {
        format!(
            "<!doctype html><html><head><title>Download Export</title></head><body><h2>Download Export</h2><div>Your export is ready.</div><a id=\"download-link\" href=\"/{account}/exports/{id}/download\">Download your data</a></body></html>"
        )
    } else {
        "<!doctype html><html><head><title>Download Expired</title></head><body><h2>Download Expired</h2><div>That download link has expired.</div></body></html>".to_owned()
    };
    Ok(Html(html).into_response())
}

pub(super) async fn download(
    State(app): Shared,
    Path((account, export_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let row = sqlx::query("SELECT status,user_id,kind FROM exports WHERE id=? AND account_id=?")
        .bind(&export_id)
        .bind(&p.account_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)?;
    if row.get::<String, _>("user_id") != p.user_id {
        return Err(missing());
    }
    if row.get::<String, _>("status") != "completed" {
        return Err(missing());
    }
    let file = tokio::fs::File::open(export_path(&export_id))
        .await
        .map_err(|_| missing())?;
    let filename = if row.get::<String, _>("kind") == "account" {
        format!("fizzy-account-{}-export-{export_id}.zip", p.account_id)
    } else {
        format!("fizzy-user-data-export-{export_id}.zip")
    };
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        Body::from_stream(ReaderStream::new(file)),
    )
        .into_response())
}

async fn export_card(
    app: &App,
    row: &DbRow,
) -> std::result::Result<(Value, Vec<ExportBlob>), sqlx::Error> {
    let card_id: String = row.get("id");
    let number: i64 = row.get("number");
    let attachment_rows = sqlx::query("SELECT b.attachable_sgid,b.filename,b.content_type,b.storage_key FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.account_id=? AND a.name='embed' AND ((a.record_type='Card' AND a.record_id=?) OR (a.record_type='Comment' AND a.record_id IN (SELECT id FROM comments WHERE card_id=?))) GROUP BY b.id ORDER BY b.id")
        .bind(row.get::<String, _>("account_id"))
        .bind(&card_id)
        .bind(&card_id)
        .fetch_all(&app.db).await?;
    let blobs: Vec<ExportBlob> = attachment_rows
        .iter()
        .map(|blob| {
            let key: String = blob.get("storage_key");
            let filename: String = blob.get("filename");
            ExportBlob {
                sgid: blob.get("attachable_sgid"),
                archive_path: format!("{number}/{key}_{}", archive_filename(&filename)),
                storage_path: storage::storage_dir().join(&key),
                filename,
                content_type: blob.get("content_type"),
            }
        })
        .collect();
    let blob_map: HashMap<String, &ExportBlob> =
        blobs.iter().map(|blob| (blob.sgid.clone(), blob)).collect();
    let creator = json!({
        "id": row.get::<String, _>("creator_id"),
        "name": row.get::<String, _>("creator_name"),
        "email": row.get::<Option<String>, _>("creator_email"),
    });
    let status = if row.get::<i64, _>("closed") != 0 {
        "Done".to_owned()
    } else if row.get::<i64, _>("postponed") != 0 {
        "Not now".to_owned()
    } else {
        row.get::<Option<String>, _>("column_name")
            .unwrap_or_else(|| "Maybe?".to_owned())
    };
    let comments = sqlx::query("SELECT c.id,c.body,c.created_at,u.id creator_id,u.name creator_name,u.role creator_role,CASE WHEN u.role='system' THEN NULL ELSE i.email_address END creator_email FROM comments c JOIN users u ON u.id=c.creator_id LEFT JOIN identities i ON i.id=u.identity_id WHERE c.card_id=? ORDER BY c.created_at ASC,c.rowid ASC")
        .bind(card_id).fetch_all(&app.db).await?;
    let comments = comments
        .iter()
        .map(|c| {
            let body: String = c.get("body");
            json!({
                "id":c.get::<String,_>("id"),
                "body":export_rich_text(&body, &blob_map),
                "creator":{
                    "id":c.get::<String,_>("creator_id"),
                    "name":c.get::<String,_>("creator_name"),
                    "email":c.get::<Option<String>,_>("creator_email")
                },
                "created_at":iso(&c.get::<String,_>("created_at"))
            })
        })
        .collect::<Vec<_>>();
    let description: String = row.get("description");
    Ok((
        json!({
            "number":number,
            "title":row.get::<String,_>("title"),
            "board":row.get::<String,_>("board_name"),
            "status":status,
            "creator":creator,
            "description":export_rich_text(&description, &blob_map),
            "created_at":iso(&row.get::<String,_>("created_at")),
            "updated_at":iso(&row.get::<String,_>("updated_at")),
            "comments":comments
        }),
        blobs,
    ))
}

#[derive(Debug)]
pub(crate) enum ArchiveEntry {
    Json(String, Vec<u8>),
    Blob(String, PathBuf),
}

pub(crate) fn write_archive(
    path: PathBuf,
    mut receiver: tokio::sync::mpsc::Receiver<ArchiveEntry>,
) -> std::io::Result<()> {
    let temporary = path.with_extension("zip.tmp");
    let result = (|| {
        std::fs::create_dir_all(path.parent().expect("export parent"))?;
        let file = std::fs::File::create(&temporary)?;
        let mut writer = ZipWriter::new(file);
        while let Some(entry) = receiver.blocking_recv() {
            match entry {
                ArchiveEntry::Json(name, data) => {
                    let options = SimpleFileOptions::default()
                        .compression_method(CompressionMethod::Deflated);
                    writer.start_file(name, options)?;
                    writer.write_all(&data)?;
                }
                ArchiveEntry::Blob(name, path) => {
                    let mut source = match std::fs::File::open(path) {
                        Ok(file) => file,
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                        Err(error) => return Err(error),
                    };
                    let options =
                        SimpleFileOptions::default().compression_method(CompressionMethod::Stored);
                    writer.start_file(name, options)?;
                    std::io::copy(&mut source, &mut writer)?;
                }
            }
        }
        writer.finish()?;
        std::fs::rename(&temporary, &path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(temporary);
    }
    result
}

async fn build_user(
    app: &App,
    row: &DbRow,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let export_id: String = row.get("id");
    let account_id: String = row.get("account_id");
    let user_id: String = row.get("user_id");
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let path = export_path(&export_id);
    let writer = tokio::task::spawn_blocking(move || write_archive(path, receiver));
    let build_result = async {
        let mut offset = 0_i64;
        loop {
        let rows = sqlx::query("SELECT c.*,b.name board_name,k.name column_name,u.name creator_name,CASE WHEN u.role='system' THEN NULL ELSE i.email_address END creator_email FROM cards c JOIN boards b ON b.id=c.board_id JOIN users u ON u.id=c.creator_id LEFT JOIN identities i ON i.id=u.identity_id LEFT JOIN columns k ON k.id=c.column_id WHERE c.account_id=? AND EXISTS(SELECT 1 FROM accesses a WHERE a.board_id=c.board_id AND a.user_id=?) ORDER BY c.id LIMIT 100 OFFSET ?")
            .bind(&account_id).bind(&user_id).bind(offset)
            .fetch_all(&app.db).await?;
        if rows.is_empty() { break; }
        offset += rows.len() as i64;
        sqlx::query("UPDATE exports SET updated_at=? WHERE id=?")
            .bind(now()).bind(&export_id).execute(&app.db).await?;
        for card in &rows {
            let (value, blobs) = export_card(app, card).await?;
            let number: i64 = card.get("number");
            let bytes = serde_json::to_vec_pretty(&value)?;
            sender.send(ArchiveEntry::Json(format!("{number}.json"),bytes)).await?;
            for blob in blobs {
                sender.send(ArchiveEntry::Blob(blob.archive_path, blob.storage_path)).await?;
            }
        }
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }.await;
    drop(sender);
    let archive_result = writer.await?;
    if let Err(error) = build_result {
        let _ = tokio::fs::remove_file(export_path(&export_id)).await;
        return Err(error);
    }
    archive_result?;
    Ok(())
}

async fn send_completion_email(app: &App, row: &DbRow) -> Result<()> {
    let Ok(sendmail) = env::var("RIZZY_SENDMAIL") else {
        return Ok(());
    };
    let from = env::var("RIZZY_MAIL_FROM").unwrap_or_else(|_| "no-reply@localhost".into());
    let email: String = sqlx::query_scalar(
        "SELECT i.email_address FROM users u JOIN identities i ON i.id=u.identity_id WHERE u.id=?",
    )
    .bind(row.get::<String, _>("user_id"))
    .fetch_one(&app.db)
    .await
    .map_err(db_err)?;
    if !valid_email(&email) || from.contains(['\r', '\n']) {
        return Err(bad("Invalid mail address"));
    }
    let origin = env::var("RIZZY_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3007".into());
    let account: i64 = sqlx::query_scalar("SELECT external_account_id FROM accounts WHERE id=?")
        .bind(row.get::<String, _>("account_id"))
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let export_id: String = row.get("id");
    let user_id: String = row.get("user_id");
    let path = if row.get::<String, _>("kind") == "account" {
        format!("/{account}/account/exports/{export_id}")
    } else {
        format!("/{account}/users/{user_id}/data_exports/{export_id}")
    };
    let message = format!(
        "From: {from}\nTo: {email}\nSubject: Your Rizzy data export is ready for download\nContent-Type: text/plain; charset=UTF-8\n\nYour data export is ready. Download it at:\n{origin}{path}\n"
    );
    let mut child = Command::new(sendmail)
        .arg("-t")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"))?;
    let Some(mut stdin) = child.stdin.take() else {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"));
    };
    stdin
        .write_all(message.as_bytes())
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"))?;
    drop(stdin);
    if !child
        .wait()
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"))?
        .success()
    {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"));
    }
    Ok(())
}

async fn run_one(
    app: &App,
    row: &DbRow,
) -> std::result::Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match row.get::<String, _>("kind").as_str() {
        "user" => build_user(app, row).await,
        "account" => account_transfer::build(app, row).await,
        _ => Err("Unsupported export kind".into()),
    }
}

pub(super) async fn worker(app: Arc<App>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let mut ticks = 0_u64;
    loop {
        let timer_tick = tokio::select! {
            _ = interval.tick() => true,
            _ = app.export_notify.notified() => false,
        };
        if timer_tick {
            ticks += 1;
        }
        if timer_tick && (ticks == 1 || ticks.is_multiple_of(60)) {
            let stale = (Utc::now() - chrono::Duration::minutes(5))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            if let Err(error) = sqlx::query(
                "UPDATE exports SET status='pending' WHERE status='processing' AND updated_at<?",
            )
            .bind(stale)
            .execute(&app.db)
            .await
            {
                eprintln!("export recovery failed: {error}");
            }
        }
        if timer_tick && (ticks == 1 || ticks.is_multiple_of(3600)) {
            let expired = (Utc::now() - chrono::Duration::hours(24))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            if let Ok(rows) = sqlx::query("SELECT id FROM exports WHERE completed_at<?")
                .bind(expired)
                .fetch_all(&app.db)
                .await
            {
                for row in rows {
                    let id: String = row.get("id");
                    let path = export_path(&id);
                    match tokio::fs::remove_file(&path).await {
                        Ok(()) => {
                            let _ = sqlx::query("DELETE FROM exports WHERE id=?")
                                .bind(id)
                                .execute(&app.db)
                                .await;
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                            let _ = sqlx::query("DELETE FROM exports WHERE id=?")
                                .bind(id)
                                .execute(&app.db)
                                .await;
                        }
                        Err(error) => eprintln!("export cleanup failed: {error}"),
                    }
                }
            }
        }
        let pending =
            sqlx::query("SELECT * FROM exports WHERE status='pending' ORDER BY created_at LIMIT 1")
                .fetch_optional(&app.db)
                .await;
        let Some(row) = (match pending {
            Ok(row) => row,
            Err(error) => {
                eprintln!("export queue read failed: {error}");
                continue;
            }
        }) else {
            continue;
        };
        let export_id: String = row.get("id");
        let claimed = sqlx::query(
            "UPDATE exports SET status='processing',updated_at=? WHERE id=? AND status='pending'",
        )
        .bind(now())
        .bind(&export_id)
        .execute(&app.db)
        .await;
        if !claimed.is_ok_and(|r| r.rows_affected() == 1) {
            continue;
        }
        let mut job = Box::pin(run_one(&app, &row));
        let outcome = loop {
            tokio::select! {
                result = &mut job => break result,
                _ = tokio::time::sleep(Duration::from_secs(30)) => {
                    if let Err(error) = sqlx::query("UPDATE exports SET updated_at=? WHERE id=? AND status='processing'")
                        .bind(now()).bind(&export_id).execute(&app.db).await {
                        eprintln!("export heartbeat failed: {error}");
                    }
                }
            }
        };
        match outcome {
            Ok(()) => {
                if let Err(error) = sqlx::query(
                    "UPDATE exports SET status='completed',completed_at=?,updated_at=? WHERE id=?",
                )
                .bind(now())
                .bind(now())
                .bind(&export_id)
                .execute(&app.db)
                .await
                {
                    eprintln!("export completion failed: {error}");
                } else if let Err(error) = send_completion_email(&app, &row).await {
                    eprintln!("export completion email failed: {}", error.1);
                }
            }
            Err(error) => {
                eprintln!("export {export_id} failed: {error}");
                let _ = sqlx::query("UPDATE exports SET status='failed',updated_at=? WHERE id=?")
                    .bind(now())
                    .bind(&export_id)
                    .execute(&app.db)
                    .await;
            }
        }
        app.export_notify.notify_one();
    }
}
