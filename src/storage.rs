use super::*;
use axum::{body::Body, http::Request};
use base64::Engine as _;
use futures_util::StreamExt;
use md5::Md5;
use std::path::PathBuf;
use std::sync::OnceLock;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

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

fn embed_html(blob: &SqliteRow, sgid: &str, slug: i64, in_paragraph: bool) -> String {
    let filename: String = blob.get("filename");
    let content_type: String = blob.get("content_type");
    let size: i64 = blob.get("byte_size");
    let signed_id: String = blob.get("signed_id");
    let path = format!(
        "/{slug}/rails/active_storage/blobs/redirect/{signed_id}/{}",
        encoded_filename(&filename)
    );
    let variant = format!(
        "/{slug}/rails/active_storage/representations/redirect/{signed_id}/preview/{}",
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
        let width = blob.get::<Option<i64>, _>("width").unwrap_or(0);
        let height = blob.get::<Option<i64>, _>("height").unwrap_or(0);
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
            plain_input.push_str(&format!("[{}]", filename));
            replacements.push((
                token.clone(),
                embed_html(&blob, value, slug, true),
                embed_html(&blob, value, slug, false),
            ));
            attached = true;
        } else {
            let mut user = sqlx::query("SELECT u.name FROM user_attachables s JOIN users u ON u.id=s.user_id WHERE s.sgid=? AND s.account_id=?")
                .bind(value).bind(account_id).fetch_optional(&app.db).await.map_err(db_err)?;
            if user.is_none() {
                if let Some(user_id) = value.strip_prefix("gid://fizzy/User/") {
                    user = sqlx::query("SELECT name FROM users WHERE id=? AND account_id=?")
                        .bind(user_id)
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
    tx: &mut sqlx::Transaction<'_, Sqlite>,
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
    let keys = purge_card_images_tx(tx, &p.account_id, Some(card_id), None).await?;
    if let Some(blob_id) = blob_id {
        sqlx::query("INSERT INTO storage_attachments(id,account_id,blob_id,record_type,record_id,name,created_at) VALUES(?,?,?,'Card',?,'image',?)")
            .bind(id()).bind(&p.account_id).bind(blob_id).bind(card_id).bind(now())
            .execute(&mut **tx).await.map_err(db_err)?;
    }
    Ok(keys)
}

pub(super) struct PendingUpload {
    pub signed_id: String,
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
    let mut pending = PendingUpload {
        signed_id: random_token(),
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
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    p: &Principal,
    upload: &PendingUpload,
) -> Result<()> {
    let timestamp = now();
    sqlx::query("INSERT INTO storage_blobs(id,account_id,storage_key,filename,content_type,byte_size,checksum,signed_id,attachable_sgid,upload_token_hash,upload_expires_at,uploaded_at,width,height,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
        .bind(id()).bind(&p.account_id).bind(upload.key.as_deref()).bind(&upload.filename).bind(&upload.content_type)
        .bind(upload.byte_size)
        .bind(&upload.checksum).bind(&upload.signed_id).bind(random_token())
        .bind(hash(&random_token())).bind(expires_in(60)).bind(&timestamp)
        .bind(upload.dimensions.map(|(width, _)| width)).bind(upload.dimensions.map(|(_, height)| height))
        .bind(&timestamp).execute(&mut **tx).await.map_err(db_err)?;
    Ok(())
}

pub(super) async fn purge_card_images_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    account_id: &str,
    card_id: Option<&str>,
    board_id: Option<&str>,
) -> Result<Vec<String>> {
    let rows = sqlx::query("SELECT a.id AS attachment_id,b.id AS blob_id,b.storage_key FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id JOIN cards c ON c.id=a.record_id WHERE a.record_type='Card' AND a.name='image' AND c.account_id=? AND (? IS NULL OR c.id=?) AND (? IS NULL OR c.board_id=?)")
        .bind(account_id)
        .bind(card_id).bind(card_id)
        .bind(board_id).bind(board_id)
        .fetch_all(&mut **tx).await.map_err(db_err)?;
    let mut keys = Vec::new();
    for row in rows {
        let attachment_id: String = row.get("attachment_id");
        let blob_id: String = row.get("blob_id");
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
            sqlx::query("DELETE FROM storage_blobs WHERE id=?")
                .bind(&blob_id)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;
            keys.push(row.get("storage_key"));
        }
    }
    Ok(keys)
}

pub(super) async fn sync_embeds_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    p: &Principal,
    record_type: &str,
    record_id: &str,
    content: &str,
) -> Result<Vec<String>> {
    if !matches!(record_type, "Card" | "Comment") {
        return Err(bad("Invalid embedded attachment owner"));
    }
    let mut wanted = HashSet::new();
    for sgid in rich_text::attachment_sgids(content) {
        let blob: Option<String> = sqlx::query_scalar("SELECT id FROM storage_blobs WHERE attachable_sgid=? AND account_id=? AND uploaded_at IS NOT NULL")
            .bind(sgid).bind(&p.account_id).fetch_optional(&mut **tx).await.map_err(db_err)?;
        if let Some(blob) = blob {
            wanted.insert(blob);
        }
    }
    let existing = sqlx::query("SELECT a.id AS attachment_id,a.blob_id,b.storage_key FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.record_type=? AND a.record_id=? AND a.name='embed' AND a.account_id=?")
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
            sqlx::query("DELETE FROM storage_blobs WHERE id=?")
                .bind(blob_id)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;
            keys.push(row.get("storage_key"));
        }
    }
    for blob_id in wanted.difference(&current) {
        sqlx::query("INSERT INTO storage_attachments(id,account_id,blob_id,record_type,record_id,name,created_at) VALUES(?,?,?,?,?,'embed',?)")
            .bind(id()).bind(&p.account_id).bind(blob_id).bind(record_type).bind(record_id).bind(now())
            .execute(&mut **tx).await.map_err(db_err)?;
    }
    Ok(keys)
}

pub(super) async fn purge_card_storage_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    account_id: &str,
    card_id: Option<&str>,
    board_id: Option<&str>,
) -> Result<Vec<String>> {
    let rows = sqlx::query("SELECT a.id AS attachment_id,a.blob_id,b.storage_key FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.account_id=? AND ((a.record_type='Card' AND a.record_id IN (SELECT id FROM cards WHERE account_id=? AND (? IS NULL OR id=?) AND (? IS NULL OR board_id=?))) OR (a.record_type='Comment' AND a.record_id IN (SELECT m.id FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.account_id=? AND (? IS NULL OR c.id=?) AND (? IS NULL OR c.board_id=?))))")
        .bind(account_id).bind(account_id).bind(card_id).bind(card_id).bind(board_id).bind(board_id)
        .bind(account_id).bind(card_id).bind(card_id).bind(board_id).bind(board_id)
        .fetch_all(&mut **tx).await.map_err(db_err)?;
    let mut keys = Vec::new();
    for row in rows {
        let blob_id: String = row.get("blob_id");
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
            sqlx::query("DELETE FROM storage_blobs WHERE id=?")
                .bind(blob_id)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;
            keys.push(row.get("storage_key"));
        }
    }
    Ok(keys)
}

pub(super) async fn remove_files(keys: Vec<String>) {
    for key in keys {
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
    tx: &mut sqlx::Transaction<'_, Sqlite>,
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
    let keys = purge_user_avatar_tx(tx, &p.account_id, user_id).await?;
    if let Some(blob_id) = blob_id {
        sqlx::query("INSERT INTO storage_attachments(id,account_id,blob_id,record_type,record_id,name,created_at) VALUES(?,?,?,'User',?,'avatar',?)")
            .bind(id()).bind(&p.account_id).bind(blob_id).bind(user_id).bind(now())
            .execute(&mut **tx).await.map_err(db_err)?;
    }
    Ok(keys)
}

pub(super) async fn purge_user_avatar_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    account_id: &str,
    user_id: &str,
) -> Result<Vec<String>> {
    let row = sqlx::query("SELECT a.id AS attachment_id,b.id AS blob_id,b.storage_key FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.record_type='User' AND a.name='avatar' AND a.record_id=? AND a.account_id=?")
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
    sqlx::query("DELETE FROM storage_blobs WHERE id=?")
        .bind(blob_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    Ok(vec![row.get("storage_key")])
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
    let signed_id = random_token();
    let attachable_sgid = random_token();
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

pub(super) async fn show_blob(
    State(app): Shared,
    Path((account, signed_id, _filename)): Path<(String, String, String)>,
    Query(query): Query<HashMap<String, String>>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Result<Response> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let blob = sqlx::query("SELECT b.* FROM storage_blobs b JOIN accounts a ON a.id=b.account_id WHERE b.signed_id=? AND a.external_account_id=?")
        .bind(&signed_id).bind(slug).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    if blob.get::<Option<String>, _>("uploaded_at").is_none() {
        return Err(missing());
    }
    let variant = if uri.path().contains("/representations/redirect/") {
        if !variable_image(&blob.get::<String, _>("content_type")) {
            return Err(missing());
        }
        "large"
    } else {
        "original"
    };
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
        let user_id = storage_user_id(&app, &headers, &account_id).await?;
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
    Ok((StatusCode::FOUND, [(header::LOCATION, location)]).into_response())
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

pub(super) async fn download_file(
    State(app): Shared,
    Path((account, token, _filename)): Path<(String, String, String)>,
) -> Result<Response> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let blob = sqlx::query("SELECT b.*,t.disposition,t.variant FROM storage_download_tokens t JOIN storage_blobs b ON b.id=t.blob_id JOIN accounts a ON a.id=b.account_id WHERE t.token_hash=? AND a.external_account_id=? AND t.expires_at>=?")
        .bind(hash(&token)).bind(slug).bind(now()).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    let key: String = blob.get("storage_key");
    let declared: String = blob.get("content_type");
    let variant: String = blob.get("variant");
    let path = if variant == "large" {
        ensure_large_variant(&key, &declared).await?
    } else {
        storage_dir().join(&key)
    };
    let file = fs::File::open(&path).await.map_err(|_| missing())?;
    let length = file.metadata().await.map_err(|_| missing())?.len();
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
    let mut response = Body::from_stream(ReaderStream::new(file)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=300"),
    );
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string()).expect("nonnegative blob size"),
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
        if dangerous || blob.get::<String, _>("disposition") == "attachment" {
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
        let location = format!(
            "{}/{account}/rails/active_storage/representations/redirect/{signed_id}/thumb/{}?disposition=inline",
            base(&headers),
            encoded_filename(&filename)
        );
        return Ok((StatusCode::FOUND, [(header::LOCATION, location)]).into_response());
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
    let keys = purge_user_avatar_tx(&mut tx, &p.account_id, &user_id).await?;
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
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let keys = purge_card_images_tx(&mut tx, &p.account_id, Some(&card_id), None).await?;
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
