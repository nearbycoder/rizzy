use super::*;
use axum::{body::Body, http::Request};
use base64::Engine as _;
use futures_util::StreamExt;
use md5::Md5;
use std::path::PathBuf;
use tokio::fs;
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;

fn storage_dir() -> PathBuf {
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

pub(super) async fn remove_files(keys: Vec<String>) {
    for key in keys {
        if let Err(error) = fs::remove_file(storage_dir().join(&key)).await {
            if error.kind() != std::io::ErrorKind::NotFound {
                eprintln!("storage cleanup failed for {key}: {error}");
            }
        }
    }
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
    sqlx::query("UPDATE storage_blobs SET uploaded_at=? WHERE id=?")
        .bind(now())
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
    headers: HeaderMap,
) -> Result<Response> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let blob = sqlx::query("SELECT b.* FROM storage_blobs b JOIN accounts a ON a.id=b.account_id WHERE b.signed_id=? AND a.external_account_id=?")
        .bind(&signed_id).bind(slug).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    if blob.get::<Option<String>, _>("uploaded_at").is_none() {
        return Err(missing());
    }
    let account_id: String = blob.get("account_id");
    let attachment = sqlx::query(
        "SELECT record_type,record_id FROM storage_attachments WHERE blob_id=? LIMIT 1",
    )
    .bind(blob.get::<String, _>("id"))
    .fetch_optional(&app.db)
    .await
    .map_err(db_err)?;
    let card = if let Some(attachment) = &attachment {
        if attachment.get::<String, _>("record_type") == "Card" {
            sqlx::query("SELECT c.status,c.board_id,b.publication_key FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.id=? AND c.account_id=?")
                .bind(attachment.get::<String, _>("record_id")).bind(&account_id)
                .fetch_optional(&app.db).await.map_err(db_err)?
        } else {
            None
        }
    } else {
        None
    };
    let public = card.as_ref().is_some_and(|card| {
        card.get::<String, _>("status") == "published"
            && card.get::<Option<String>, _>("publication_key").is_some()
    });
    if !public {
        let user_id = storage_user_id(&app, &headers, &account_id).await?;
        if let Some(card) = card {
            let access: i64 =
                sqlx::query_scalar("SELECT COUNT(*) FROM accesses WHERE board_id=? AND user_id=?")
                    .bind(card.get::<String, _>("board_id"))
                    .bind(user_id)
                    .fetch_one(&app.db)
                    .await
                    .map_err(db_err)?;
            if access == 0 {
                return Err(err(StatusCode::FORBIDDEN, "Board access required"));
            }
        } else if attachment.is_some() {
            return Err(err(StatusCode::FORBIDDEN, "Blob access required"));
        }
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
        "{}/{account}/rails/active_storage/disk/{token}/{}",
        base(&headers),
        encoded_filename(&filename)
    );
    Ok((StatusCode::FOUND, [(header::LOCATION, location)]).into_response())
}

pub(super) async fn download_file(
    State(app): Shared,
    Path((account, token, _filename)): Path<(String, String, String)>,
) -> Result<Response> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let blob = sqlx::query("SELECT b.* FROM storage_download_tokens t JOIN storage_blobs b ON b.id=t.blob_id JOIN accounts a ON a.id=b.account_id WHERE t.token_hash=? AND a.external_account_id=? AND t.expires_at>=?")
        .bind(hash(&token)).bind(slug).bind(now()).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    let key: String = blob.get("storage_key");
    let file = fs::File::open(storage_dir().join(key))
        .await
        .map_err(|_| missing())?;
    let declared: String = blob.get("content_type");
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
        HeaderValue::from_str(&blob.get::<i64, _>("byte_size").to_string())
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
        if dangerous { "attachment" } else { "inline" },
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
