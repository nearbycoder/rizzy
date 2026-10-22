use super::*;
use axum::extract::Multipart;
use base64::Engine as _;
use md5::Md5;
use std::{
    collections::{BTreeMap, HashMap},
    io::Read,
    path::PathBuf,
};
use zip::ZipArchive;

type ImportResult<T> = std::result::Result<T, String>;

fn import_dir() -> PathBuf {
    env::var_os("RIZZY_IMPORT_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("data/imports"))
}

pub(crate) fn import_path(import_id: &str) -> PathBuf {
    import_dir().join(format!("{import_id}.zip"))
}

fn wants_json(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("json"))
}

pub(super) async fn new(State(app): Shared, headers: HeaderMap) -> Result<Response> {
    identity_auth(&app, &headers).await?;
    Ok(Html("<!doctype html><html><head><title>Import an account</title></head><body><main><h1>Import a Fizzy account</h1><form action=\"/account/imports\" method=\"post\" enctype=\"multipart/form-data\"><label>Choose a Fizzy account ZIP <input type=\"file\" name=\"file\" accept=\".zip\" required></label><button type=\"submit\">Start Import</button></form></main></body></html>").into_response())
}

pub(super) async fn create(
    State(app): Shared,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Result<Response> {
    let (identity_id, permission) = identity_auth(&app, &headers).await?;
    if permission != "write" {
        return Err(err(StatusCode::FORBIDDEN, "Write permission required"));
    }
    let import_id = id();
    tokio::fs::create_dir_all(import_dir())
        .await
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Cannot store import"))?;
    let path = import_path(&import_id);
    let mut upload_cleanup = FileCleanup(vec![path.clone()]);
    let mut found_file = false;
    let mut bytes_written = 0_u64;
    while let Some(mut field) = multipart
        .next_field()
        .await
        .map_err(|_| bad("Invalid upload"))?
    {
        if field.name() != Some("file") || found_file {
            continue;
        }
        found_file = true;
        let mut out = tokio::fs::File::create(&path)
            .await
            .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Cannot store import"))?;
        while let Some(chunk) = field.chunk().await.map_err(|_| bad("Invalid upload"))? {
            bytes_written += chunk.len() as u64;
            if bytes_written > 512 * 1024 * 1024 {
                let _ = tokio::fs::remove_file(&path).await;
                return Err(bad("Import ZIP is too large"));
            }
            out.write_all(&chunk)
                .await
                .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Cannot store import"))?;
        }
        out.flush()
            .await
            .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Cannot store import"))?;
    }
    if !found_file || bytes_written == 0 {
        return Err(bad("file is required"));
    }
    let _guard = app.write_lock.lock().await;
    let owner = sqlx::query("SELECT u.name,i.email_address FROM identities i LEFT JOIN users u ON u.identity_id=i.id WHERE i.id=? ORDER BY u.created_at LIMIT 1")
        .bind(&identity_id).fetch_one(&app.db).await.map_err(db_err)?;
    let email: String = owner.get("email_address");
    let owner_name: String = owner
        .try_get::<Option<String>, _>("name")
        .map_err(db_err)?
        .unwrap_or_else(|| email.split('@').next().unwrap_or("Importer").to_owned());
    let account_id = id();
    let user_id = id();
    let system_id = id();
    let timestamp = now();
    let slug = account_sequence::next(&app.db, app.mysql)
        .await
        .map_err(db_err)?;
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query(
        "INSERT INTO accounts(id,external_account_id,name,created_at) VALUES(?,?,'Import',?)",
    )
    .bind(&account_id)
    .bind(slug)
    .bind(&timestamp)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
    insert_join_code_tx(&mut tx, &account_id, app.mysql)
        .await
        .map_err(db_err)?;
    sqlx::query("INSERT INTO users(id,account_id,identity_id,name,role,created_at,verified_at) VALUES(?,?,?,?,'owner',?,?)")
        .bind(&user_id).bind(&account_id).bind(&identity_id).bind(owner_name).bind(&timestamp).bind(&timestamp)
        .execute(&mut *tx).await.map_err(db_err)?;
    sqlx::query(
        "INSERT INTO users(id,account_id,name,role,created_at) VALUES(?,?,'System','system',?)",
    )
    .bind(&system_id)
    .bind(&account_id)
    .bind(&timestamp)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
    sqlx::query(
        "INSERT INTO imports(id,account_id,identity_id,created_at,updated_at) VALUES(?,?,?,?,?)",
    )
    .bind(&import_id)
    .bind(&account_id)
    .bind(&identity_id)
    .bind(&timestamp)
    .bind(&timestamp)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    upload_cleanup.0.clear();
    app.import_notify.notify_one();
    let location = format!("/{slug}/account/imports/{import_id}");
    if wants_json(&headers) {
        Ok((StatusCode::CREATED, Json(json!({"id":import_id,"status":"pending","account":slug,"url":format!("{}{}",base(&headers),location)}))).into_response())
    } else {
        Ok((StatusCode::FOUND, [(header::LOCATION, location)]).into_response())
    }
}

pub(super) async fn show(
    State(app): Shared,
    Path((account, import_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let row = sqlx::query("SELECT * FROM imports WHERE id=? AND account_id=?")
        .bind(import_id.trim_end_matches(".json"))
        .bind(&p.account_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)?;
    if row.get::<String, _>("identity_id") != identity_auth(&app, &headers).await?.0 {
        return Err(err(StatusCode::FORBIDDEN, "Import owner required"));
    }
    let status: String = row.get("status");
    let failure_reason: Option<String> = row.get("failure_reason");
    if wants_json(&headers) || import_id.ends_with(".json") {
        return Ok(Json(json!({"id":row.get::<String,_>("id"),"status":status,"failure_reason":failure_reason,"account":p.slug})).into_response());
    }
    let detail = match status.as_str() {
        "completed" => format!(
            "<strong>Your import was successful!</strong><a href=\"/{}/\">Go to your account</a>",
            p.slug
        ),
        "failed" => format!(
            "Import failed: {}",
            escape(failure_reason.as_deref().unwrap_or("unexpected_error"))
        ),
        _ => "Your import is in progress. This may take a while for large accounts.".to_owned(),
    };
    Ok(Html(format!("<!doctype html><html><head><title>Import status</title></head><body><main><h1>Import status</h1><div>{detail}</div></main></body></html>")).into_response())
}

struct Archive {
    account: Value,
    groups: BTreeMap<String, Vec<Value>>,
    storage: BTreeMap<String, Vec<u8>>,
}

fn load_archive(path: &PathBuf) -> ImportResult<Archive> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = ZipArchive::new(file).map_err(|e| format!("Invalid ZIP: {e}"))?;
    if zip.len() > 1_000_000 {
        return Err("Too many ZIP entries".into());
    }
    let mut total = 0_u64;
    let mut account = None;
    let mut groups = BTreeMap::<String, Vec<Value>>::new();
    let mut storage = BTreeMap::<String, Vec<u8>>::new();
    let mut seen_names = HashSet::<String>::new();
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| e.to_string())?;
        if entry.is_dir() {
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > 512 * 1024 * 1024 {
            return Err("ZIP expands beyond the import size limit".into());
        }
        let name = entry.name().to_owned();
        if !seen_names.insert(name.clone()) {
            return Err(format!("Duplicate ZIP entry: {name}"));
        }
        if name == "data/account.json" {
            if account.is_some() {
                return Err("Duplicate account record".into());
            }
            account = Some(serde_json::from_reader(&mut entry).map_err(|e| e.to_string())?);
        } else if let Some(path) = name.strip_prefix("data/") {
            let Some((group, filename)) = path.split_once('/') else {
                return Err(format!("Invalid data path: {name}"));
            };
            let Some(expected_id) = filename.strip_suffix(".json") else {
                return Err(format!("Invalid data filename: {name}"));
            };
            if expected_id.is_empty() || expected_id.contains('/') {
                return Err(format!("Invalid data filename: {name}"));
            }
            let value: Value = serde_json::from_reader(&mut entry).map_err(|e| e.to_string())?;
            if value.get("id").and_then(Value::as_str) != Some(expected_id) {
                return Err(format!("Record ID mismatch: {name}"));
            }
            groups.entry(group.to_owned()).or_default().push(value);
        } else if let Some(key) = name.strip_prefix("storage/") {
            if key.contains('\\')
                || key.contains('\0')
                || key
                    .split('/')
                    .any(|segment| segment.is_empty() || segment == "." || segment == "..")
            {
                return Err(format!("Invalid storage path: {name}"));
            }
            let mut bytes = Vec::with_capacity(entry.size() as usize);
            entry.read_to_end(&mut bytes).map_err(|e| e.to_string())?;
            if storage.insert(key.to_owned(), bytes).is_some() {
                return Err(format!("Duplicate storage file: {name}"));
            }
        } else {
            return Err(format!("Unexpected ZIP entry: {name}"));
        }
    }
    let account = account.ok_or("Missing data/account.json")?;
    let account_id = field(&account, "id")?;
    field(&account, "name")?;
    let join = account.get("join_code").ok_or("Missing join_code")?;
    field(join, "code")?;
    for (group, values) in &groups {
        if !matches!(
            group.as_str(),
            "users"
                | "user_settings"
                | "tags"
                | "boards"
                | "columns"
                | "entropies"
                | "board_publications"
                | "webhooks"
                | "accesses"
                | "cards"
                | "comments"
                | "steps"
                | "assignments"
                | "taggings"
                | "closures"
                | "card_goldnesses"
                | "card_not_nows"
                | "card_activity_spikes"
                | "watches"
                | "pins"
                | "reactions"
                | "mentions"
                | "filters"
                | "webhook_delinquency_trackers"
                | "events"
                | "notifications"
                | "notification_bundles"
                | "webhook_deliveries"
                | "active_storage_blobs"
                | "active_storage_variant_records"
                | "active_storage_attachments"
                | "action_text_rich_texts"
        ) {
            return Err(format!("Unsupported record group: {group}"));
        }
        for value in values {
            if field(value, "account_id")? != account_id {
                return Err(format!("{group} record belongs to another account"));
            }
        }
    }
    let archive = Archive {
        account,
        groups,
        storage,
    };
    validate_references(&archive)?;
    Ok(archive)
}

fn validate_references(archive: &Archive) -> ImportResult<()> {
    let mut publication_keys = HashSet::new();
    for publication in records(archive, "board_publications") {
        if !publication_keys.insert(field(publication, "key")?) {
            return Err("UNIQUE constraint: duplicate publication key".into());
        }
    }
    let ids: HashMap<String, HashSet<String>> = archive
        .groups
        .iter()
        .map(|(group, records)| {
            (
                group.clone(),
                records
                    .iter()
                    .filter_map(|record| maybe_field(record, "id").map(str::to_owned))
                    .collect(),
            )
        })
        .collect();
    let check = |record: &Value, field_name: &str, group: &str| -> ImportResult<()> {
        let related = field(record, field_name)?;
        if ids.get(group).is_some_and(|set| set.contains(related)) {
            Ok(())
        } else {
            Err(format!(
                "{field_name} references a record outside the archive"
            ))
        }
    };
    for (group, rows) in &archive.groups {
        for record in rows {
            match group.as_str() {
                "user_settings" => check(record, "user_id", "users")?,
                "boards" => check(record, "creator_id", "users")?,
                "columns" => check(record, "board_id", "boards")?,
                "entropies" if maybe_field(record, "container_type") == Some("Board") => {
                    check(record, "container_id", "boards")?
                }
                "board_publications" => check(record, "board_id", "boards")?,
                "webhooks" => check(record, "board_id", "boards")?,
                "accesses" => {
                    check(record, "board_id", "boards")?;
                    check(record, "user_id", "users")?;
                }
                "cards" => {
                    check(record, "board_id", "boards")?;
                    check(record, "creator_id", "users")?;
                    if maybe_field(record, "column_id").is_some() {
                        check(record, "column_id", "columns")?;
                    }
                }
                "comments" => {
                    check(record, "card_id", "cards")?;
                    check(record, "creator_id", "users")?;
                }
                "steps" => check(record, "card_id", "cards")?,
                "assignments" => {
                    check(record, "card_id", "cards")?;
                    check(record, "assignee_id", "users")?;
                    check(record, "assigner_id", "users")?;
                }
                "taggings" => {
                    check(record, "card_id", "cards")?;
                    check(record, "tag_id", "tags")?;
                }
                "closures" => {
                    check(record, "card_id", "cards")?;
                    if maybe_field(record, "user_id").is_some() {
                        check(record, "user_id", "users")?;
                    }
                }
                "card_goldnesses" => check(record, "card_id", "cards")?,
                // Fizzy exports can contain account-scoped spikes whose card
                // is absent from the same archive.
                "card_activity_spikes" => {}
                "card_not_nows" => {
                    check(record, "card_id", "cards")?;
                    if maybe_field(record, "user_id").is_some() {
                        check(record, "user_id", "users")?;
                    }
                }
                "watches" => {
                    check(record, "card_id", "cards")?;
                    check(record, "user_id", "users")?;
                }
                "pins" => {
                    check(record, "card_id", "cards")?;
                    check(record, "user_id", "users")?;
                }
                "reactions" => {
                    check(record, "reacter_id", "users")?;
                    let target = match field(record, "reactable_type")? {
                        "Card" => "cards",
                        "Comment" => "comments",
                        _ => return Err("Unsupported reaction target".into()),
                    };
                    check(record, "reactable_id", target)?;
                }
                "mentions" => {
                    check(record, "mentionee_id", "users")?;
                    check(record, "mentioner_id", "users")?;
                    let target = match field(record, "source_type")? {
                        "Card" => "cards",
                        "Comment" => "comments",
                        _ => return Err("Unsupported mention source".into()),
                    };
                    check(record, "source_id", target)?;
                }
                "filters" => check(record, "creator_id", "users")?,
                "webhook_delinquency_trackers" => check(record, "webhook_id", "webhooks")?,
                "events" => {
                    check(record, "board_id", "boards")?;
                    check(record, "creator_id", "users")?;
                    let target = match field(record, "eventable_type")? {
                        "Card" => "cards",
                        "Comment" => "comments",
                        _ => return Err("Unsupported event target".into()),
                    };
                    check(record, "eventable_id", target)?;
                }
                "notifications" => {
                    check(record, "user_id", "users")?;
                    check(record, "creator_id", "users")?;
                    check(record, "card_id", "cards")?;
                    let source = match field(record, "source_type")? {
                        "Event" => "events",
                        "Mention" => "mentions",
                        _ => return Err("Unsupported notification source".into()),
                    };
                    check(record, "source_id", source)?;
                }
                "notification_bundles" => check(record, "user_id", "users")?,
                "webhook_deliveries" => {
                    check(record, "webhook_id", "webhooks")?;
                    check(record, "event_id", "events")?;
                }
                "active_storage_attachments" => {
                    check(record, "blob_id", "active_storage_blobs")?;
                    let target = match field(record, "record_type")? {
                        "User" => "users",
                        "Board" => "boards",
                        "Card" => "cards",
                        "Comment" => "comments",
                        "ActionText::RichText" => "action_text_rich_texts",
                        "ActiveStorage::VariantRecord" => "active_storage_variant_records",
                        _ => return Err("Unsupported attachment target".into()),
                    };
                    check(record, "record_id", target)?;
                }
                "active_storage_variant_records" => {
                    check(record, "blob_id", "active_storage_blobs")?
                }
                "action_text_rich_texts" => {
                    let target = match field(record, "record_type")? {
                        "Board" => "boards",
                        "Card" => "cards",
                        "Comment" => "comments",
                        _ => return Err("Unsupported rich text target".into()),
                    };
                    check(record, "record_id", target)?;
                }
                _ => {}
            }
        }
    }
    Ok(())
}

struct FileCleanup(Vec<PathBuf>);
impl Drop for FileCleanup {
    fn drop(&mut self) {
        for path in &self.0 {
            let _ = std::fs::remove_file(path);
        }
    }
}

fn field<'a>(value: &'a Value, key: &str) -> ImportResult<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Missing {key}"))
}

fn maybe_field<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn integer(value: &Value, key: &str, default: i64) -> i64 {
    value.get(key).and_then(Value::as_i64).unwrap_or(default)
}

fn boolean(value: &Value, key: &str, default: bool) -> i64 {
    value.get(key).and_then(Value::as_bool).unwrap_or(default) as i64
}

fn bundle_status(value: &Value) -> ImportResult<i64> {
    match value.get("status") {
        Some(Value::String(value)) if value == "pending" => Ok(0),
        Some(Value::String(value)) if value == "processing" => Ok(1),
        Some(Value::String(value)) if value == "delivered" => Ok(2),
        Some(Value::Number(value)) => value
            .as_i64()
            .filter(|value| (0..=2).contains(value))
            .ok_or_else(|| "Invalid notification bundle status".into()),
        _ => Err("Invalid notification bundle status".into()),
    }
}

fn records<'a>(archive: &'a Archive, group: &str) -> &'a [Value] {
    archive.groups.get(group).map(Vec::as_slice).unwrap_or(&[])
}

async fn apply_archive(
    app: &App,
    import_id: &str,
    account_id: &str,
    archive: Archive,
) -> ImportResult<()> {
    let source_name = field(&archive.account, "name")?;
    let join = archive
        .account
        .get("join_code")
        .ok_or("Missing join_code")?;
    let join_code = field(join, "code")?;
    let imported_at = now();
    let mut tx = app.db.begin().await.map_err(|e| e.to_string())?;
    let importer_identity: String =
        sqlx::query_scalar("SELECT identity_id FROM imports WHERE account_id=?")
            .bind(account_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    let importer_email: String =
        sqlx::query_scalar("SELECT email_address FROM identities WHERE id=?")
            .bind(&importer_identity)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    if !records(&archive, "users").iter().any(|user| {
        maybe_field(user, "email_address") == Some(importer_email.as_str())
            && boolean(user, "active", true) != 0
    }) {
        return Err("Importing identity is absent from the account export".into());
    }
    sqlx::query("DELETE FROM users WHERE account_id=?")
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let cards_count = integer(&archive.account, "cards_count", 0);
    sqlx::query("UPDATE accounts SET name=?,cards_count=?,updated_at=CASE WHEN name<>? OR cards_count<>? THEN ? ELSE updated_at END WHERE id=?")
        .bind(source_name)
        .bind(cards_count)
        .bind(source_name)
        .bind(cards_count)
        .bind(&imported_at)
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    sqlx::query("UPDATE account_join_codes SET code=?,usage_count=?,usage_limit=?,updated_at=? WHERE account_id=?")
        .bind(join_code).bind(integer(join,"usage_count",0)).bind(integer(join,"usage_limit",10))
        .bind(now()).bind(account_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;

    for user in records(&archive, "users") {
        let user_id = field(user, "id")?;
        let role = field(user, "role")?;
        let identity_id: Option<String> = if let Some(email) =
            maybe_field(user, "email_address").filter(|email| !email.trim().is_empty())
        {
            let insert_identity = if app.mysql {
                "INSERT IGNORE INTO identities(id,email_address,created_at) VALUES(?,?,?)"
            } else {
                "INSERT OR IGNORE INTO identities(id,email_address,created_at) VALUES(?,?,?)"
            };
            sqlx::query(insert_identity)
                .bind(id())
                .bind(email)
                .bind(field(user, "created_at")?)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            Some(
                sqlx::query_scalar::<_, String>("SELECT id FROM identities WHERE email_address=?")
                    .bind(email)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| e.to_string())?,
            )
        } else {
            None
        };
        sqlx::query("INSERT INTO users(id,account_id,identity_id,name,role,active,verified_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)")
            .bind(user_id).bind(account_id).bind(identity_id).bind(field(user,"name")?).bind(role)
            .bind(boolean(user,"active",true)).bind(maybe_field(user,"verified_at"))
            .bind(field(user,"created_at")?).bind(field(user,"updated_at")?)
            .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for settings in records(&archive, "user_settings") {
        let user_id = field(settings, "user_id")?;
        let frequency = field(settings, "bundle_email_frequency")?;
        let update_notifications = if app.mysql {
            "INSERT INTO notification_settings(user_id,account_id,bundle_email_frequency) VALUES(?,?,?) ON DUPLICATE KEY UPDATE bundle_email_frequency=VALUES(bundle_email_frequency)"
        } else {
            "INSERT INTO notification_settings(user_id,account_id,bundle_email_frequency) VALUES(?,?,?) ON CONFLICT(user_id) DO UPDATE SET bundle_email_frequency=excluded.bundle_email_frequency"
        };
        sqlx::query(update_notifications)
            .bind(user_id)
            .bind(account_id)
            .bind(frequency)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        let update_timezone = if app.mysql {
            "INSERT INTO user_timezones(user_id,account_id,timezone_name) VALUES(?,?,?) ON DUPLICATE KEY UPDATE timezone_name=VALUES(timezone_name)"
        } else {
            "INSERT INTO user_timezones(user_id,account_id,timezone_name) VALUES(?,?,?) ON CONFLICT(user_id) DO UPDATE SET timezone_name=excluded.timezone_name"
        };
        sqlx::query(update_timezone)
            .bind(user_id)
            .bind(account_id)
            .bind(maybe_field(settings, "timezone_name"))
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        let changed = sqlx::query("UPDATE user_settings_records SET id=?,created_at=?,updated_at=? WHERE user_id=? AND account_id=?")
            .bind(field(settings, "id")?)
            .bind(field(settings, "created_at")?)
            .bind(&imported_at)
            .bind(user_id)
            .bind(account_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        if changed.rows_affected() == 0 {
            let existing: Option<String> = sqlx::query_scalar(
                "SELECT id FROM user_settings_records WHERE user_id=? AND account_id=?",
            )
            .bind(user_id)
            .bind(account_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
            if existing.is_none() {
                sqlx::query("INSERT INTO user_settings_records(id,account_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?)")
                    .bind(field(settings, "id")?)
                    .bind(account_id)
                    .bind(user_id)
                    .bind(field(settings, "created_at")?)
                    .bind(&imported_at)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| e.to_string())?;
            }
        }
    }
    // Fizzy inserts imported users in bulk, bypassing their create callbacks.
    // Rizzy's database triggers make default settings for every regular user;
    // discard those defaults when the archive has no settings for that user.
    let settings_users: HashSet<String> = records(&archive, "user_settings")
        .iter()
        .filter_map(|settings| maybe_field(settings, "user_id").map(str::to_owned))
        .collect();
    let imported_user_ids: Vec<String> =
        sqlx::query_scalar("SELECT id FROM users WHERE account_id=?")
            .bind(account_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    for user_id in imported_user_ids {
        if !settings_users.contains(&user_id) {
            for table in [
                "user_timezones",
                "notification_settings",
                "user_settings_records",
            ] {
                sqlx::query(&format!(
                    "DELETE FROM {table} WHERE user_id=? AND account_id=?"
                ))
                .bind(&user_id)
                .bind(account_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            }
        }
    }
    if !records(&archive, "users")
        .iter()
        .any(|user| maybe_field(user, "role") == Some("system"))
    {
        sqlx::query(
            "INSERT INTO users(id,account_id,name,role,created_at) VALUES(?,?,'System','system',?)",
        )
        .bind(id())
        .bind(account_id)
        .bind(now())
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }
    for tag in records(&archive, "tags") {
        sqlx::query(
            "INSERT INTO tags(id,account_id,title,created_at,updated_at) VALUES(?,?,?,?,?)",
        )
        .bind(field(tag, "id")?)
        .bind(account_id)
        .bind(field(tag, "title")?)
        .bind(field(tag, "created_at")?)
        .bind(&imported_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }
    for board in records(&archive, "boards") {
        sqlx::query("INSERT INTO boards(id,account_id,creator_id,name,all_access,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
            .bind(field(board,"id")?).bind(account_id).bind(field(board,"creator_id")?)
            .bind(field(board,"name")?).bind(boolean(board,"all_access",true))
            .bind(field(board,"created_at")?).bind(&imported_at)
            .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for column in records(&archive, "columns") {
        sqlx::query("INSERT INTO columns(id,account_id,board_id,name,color,position,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(field(column,"id")?).bind(account_id).bind(field(column,"board_id")?)
            .bind(field(column,"name")?).bind(field(column,"color")?)
            .bind(integer(column,"position",0)).bind(field(column,"created_at")?)
            .bind(&imported_at).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for entropy in records(&archive, "entropies") {
        let period = integer(entropy, "auto_postpone_period", 30 * 86400) / 86400;
        let container_type = field(entropy, "container_type")?;
        let container_id = if container_type == "Account" {
            account_id
        } else {
            field(entropy, "container_id")?
        };
        match container_type {
            "Account" => {
                sqlx::query("UPDATE accounts SET auto_postpone_period_in_days=? WHERE id=?")
                    .bind(period)
                    .bind(account_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e| e.to_string())?;
            }
            "Board" => {
                let changed = sqlx::query("UPDATE boards SET auto_postpone_period_in_days=?,entropy_override=1 WHERE id=? AND account_id=?")
                    .bind(period).bind(field(entropy,"container_id")?).bind(account_id)
                    .execute(&mut *tx).await.map_err(|e| e.to_string())?;
                if changed.rows_affected() != 1 {
                    return Err("Entropy references unknown board".into());
                }
            }
            _ => return Err("Unsupported entropy container".into()),
        }
        // Fizzy's entropy importer updates the destination account's existing
        // entropy in place, while board entropies are inserted from the archive.
        if container_type == "Board" {
            let changed = sqlx::query("UPDATE entropy_records SET id=?,created_at=?,updated_at=? WHERE account_id=? AND container_type='Board' AND container_id=?")
                .bind(field(entropy, "id")?)
                .bind(field(entropy, "created_at")?)
                .bind(field(entropy, "updated_at")?)
                .bind(account_id)
                .bind(container_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
            if changed.rows_affected() != 1 {
                return Err("Board entropy metadata was not restored".into());
            }
        }
    }
    for publication in records(&archive, "board_publications") {
        sqlx::query("INSERT INTO board_publications(id,account_id,board_id,`key`,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(field(publication, "id")?)
            .bind(account_id)
            .bind(field(publication, "board_id")?)
            .bind(field(publication, "key")?)
            .bind(field(publication, "created_at")?)
            .bind(&imported_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        let changed =
            sqlx::query("UPDATE boards SET publication_key=? WHERE id=? AND account_id=?")
                .bind(field(publication, "key")?)
                .bind(field(publication, "board_id")?)
                .bind(account_id)
                .execute(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        if changed.rows_affected() != 1 {
            return Err("Publication references unknown board".into());
        }
    }
    for webhook in records(&archive, "webhooks") {
        let actions = serde_json::to_string(
            webhook
                .get("subscribed_actions")
                .ok_or("Missing subscribed_actions")?,
        )
        .map_err(|e| e.to_string())?;
        sqlx::query("INSERT INTO webhooks(id,account_id,board_id,name,url,active,signing_secret,subscribed_actions,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?)")
            .bind(field(webhook,"id")?).bind(account_id).bind(field(webhook,"board_id")?)
            .bind(field(webhook,"name")?).bind(field(webhook,"url")?)
            .bind(boolean(webhook,"active",true)).bind(field(webhook,"signing_secret")?)
            .bind(actions).bind(field(webhook,"created_at")?).bind(&imported_at)
            .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for access in records(&archive, "accesses") {
        sqlx::query(
            "INSERT INTO accesses(board_id,user_id,involvement,accessed_at) VALUES(?,?,?,?)",
        )
        .bind(field(access, "board_id")?)
        .bind(field(access, "user_id")?)
        .bind(field(access, "involvement")?)
        .bind(maybe_field(access, "accessed_at"))
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
        let changed = sqlx::query("UPDATE access_records SET id=?,created_at=?,updated_at=? WHERE board_id=? AND user_id=?")
            .bind(field(access, "id")?)
            .bind(field(access, "created_at")?)
            .bind(&imported_at)
            .bind(field(access, "board_id")?)
            .bind(field(access, "user_id")?)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        if changed.rows_affected() != 1 {
            return Err("Access metadata was not restored".into());
        }
    }
    for card in records(&archive, "cards") {
        if let Some(due_on) = maybe_field(card, "due_on") {
            chrono::NaiveDate::parse_from_str(due_on, "%Y-%m-%d")
                .map_err(|_| "Invalid card due date")?;
        }
        sqlx::query("INSERT INTO cards(id,account_id,board_id,column_id,creator_id,number,title,status,due_on,created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(field(card,"id")?).bind(account_id).bind(field(card,"board_id")?)
            .bind(maybe_field(card,"column_id")).bind(field(card,"creator_id")?)
            .bind(integer(card,"number",0)).bind(field(card,"title")?).bind(field(card,"status")?)
            .bind(maybe_field(card,"due_on"))
            .bind(field(card,"created_at")?).bind(&imported_at)
            .bind(field(card,"last_active_at")?).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for comment in records(&archive, "comments") {
        sqlx::query("INSERT INTO comments(id,card_id,creator_id,body,created_at,updated_at) VALUES(?,?,?,'',?,?)")
            .bind(field(comment,"id")?).bind(field(comment,"card_id")?).bind(field(comment,"creator_id")?)
            .bind(field(comment,"created_at")?).bind(&imported_at)
            .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for step in records(&archive, "steps") {
        sqlx::query("INSERT INTO steps(id,card_id,content,completed,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(field(step, "id")?)
            .bind(field(step, "card_id")?)
            .bind(field(step, "content")?)
            .bind(boolean(step, "completed", false))
            .bind(field(step, "created_at")?)
            .bind(&imported_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    for assignment in records(&archive, "assignments") {
        sqlx::query("INSERT INTO assignments(id,account_id,card_id,assignee_id,assigner_id,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
            .bind(field(assignment,"id")?).bind(account_id).bind(field(assignment,"card_id")?)
            .bind(field(assignment,"assignee_id")?).bind(field(assignment,"assigner_id")?)
            .bind(field(assignment,"created_at")?).bind(&imported_at).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for tagging in records(&archive, "taggings") {
        sqlx::query(
            "INSERT INTO taggings(id,card_id,tag_id,created_at,updated_at) VALUES(?,?,?,?,?)",
        )
        .bind(field(tagging, "id")?)
        .bind(field(tagging, "card_id")?)
        .bind(field(tagging, "tag_id")?)
        .bind(field(tagging, "created_at")?)
        .bind(&imported_at)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }
    for closure in records(&archive, "closures") {
        let card_id = field(closure, "card_id")?;
        let created_at = field(closure, "created_at")?;
        sqlx::query("INSERT INTO closures(id,account_id,card_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(field(closure, "id")?)
            .bind(account_id)
            .bind(card_id)
            .bind(maybe_field(closure, "user_id"))
            .bind(created_at)
            .bind(&imported_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("UPDATE cards SET closed=1 WHERE id=? AND account_id=?")
            .bind(card_id)
            .bind(account_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    for gold in records(&archive, "card_goldnesses") {
        let card_id = field(gold, "card_id")?;
        let created_at = field(gold, "created_at")?;
        sqlx::query("INSERT INTO card_goldnesses(id,account_id,card_id,created_at,updated_at) VALUES(?,?,?,?,?)")
            .bind(field(gold, "id")?)
            .bind(account_id)
            .bind(card_id)
            .bind(created_at)
            .bind(&imported_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("UPDATE cards SET golden=1 WHERE id=? AND account_id=?")
            .bind(card_id)
            .bind(account_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    for not_now in records(&archive, "card_not_nows") {
        let card_id = field(not_now, "card_id")?;
        let created_at = field(not_now, "created_at")?;
        sqlx::query("INSERT INTO card_not_nows(id,account_id,card_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(field(not_now, "id")?)
            .bind(account_id)
            .bind(card_id)
            .bind(maybe_field(not_now, "user_id"))
            .bind(created_at)
            .bind(&imported_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
        sqlx::query("UPDATE cards SET postponed=1 WHERE id=? AND account_id=?")
            .bind(card_id)
            .bind(account_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    for spike in records(&archive, "card_activity_spikes") {
        sqlx::query("INSERT INTO card_activity_spikes(id,card_id,account_id,created_at,updated_at) VALUES(?,?,?,?,?)")
            .bind(field(spike,"id")?).bind(field(spike,"card_id")?).bind(account_id).bind(field(spike,"created_at")?)
            .bind(&imported_at).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for watch in records(&archive, "watches") {
        sqlx::query("INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(field(watch, "id")?)
            .bind(field(watch, "card_id")?)
            .bind(field(watch, "user_id")?)
            .bind(boolean(watch, "watching", true))
            .bind(field(watch, "created_at")?)
            .bind(&imported_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    for pin in records(&archive, "pins") {
        sqlx::query("INSERT INTO pins(id,account_id,card_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(field(pin, "id")?)
            .bind(account_id)
            .bind(field(pin, "card_id")?)
            .bind(field(pin, "user_id")?)
            .bind(field(pin, "created_at")?)
            .bind(&imported_at)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    for reaction in records(&archive, "reactions") {
        sqlx::query("INSERT INTO reactions(id,account_id,reactable_type,reactable_id,reacter_id,content,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(field(reaction,"id")?).bind(account_id).bind(field(reaction,"reactable_type")?)
            .bind(field(reaction,"reactable_id")?).bind(field(reaction,"reacter_id")?)
            .bind(field(reaction,"content")?).bind(field(reaction,"created_at")?).bind(&imported_at)
            .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for mention in records(&archive, "mentions") {
        sqlx::query("INSERT INTO mentions(id,account_id,mentionee_id,mentioner_id,source_type,source_id,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(field(mention,"id")?).bind(account_id).bind(field(mention,"mentionee_id")?)
            .bind(field(mention,"mentioner_id")?).bind(field(mention,"source_type")?)
            .bind(field(mention,"source_id")?).bind(field(mention,"created_at")?)
            .bind(&imported_at).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for filter in records(&archive, "filters") {
        let fields = serde_json::to_string(filter.get("fields").ok_or("Missing filter fields")?)
            .map_err(|e| e.to_string())?;
        sqlx::query("INSERT INTO filters(id,account_id,creator_id,fields,params_digest,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
            .bind(field(filter,"id")?).bind(account_id).bind(field(filter,"creator_id")?)
            .bind(fields).bind(field(filter,"params_digest")?).bind(field(filter,"created_at")?)
            .bind(&imported_at).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for tracker in records(&archive, "webhook_delinquency_trackers") {
        let webhook_id = field(tracker, "webhook_id")?;
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM webhooks WHERE id=? AND account_id=?")
                .bind(webhook_id)
                .bind(account_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| e.to_string())?;
        if exists != 1 {
            return Err("Tracker references unknown webhook".into());
        }
        let tracker_id = maybe_field(tracker, "id").unwrap_or(webhook_id);
        sqlx::query("INSERT INTO webhook_delinquency_trackers(id,account_id,webhook_id,consecutive_failures_count,first_failure_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
            .bind(tracker_id).bind(account_id).bind(webhook_id)
            .bind(integer(tracker,"consecutive_failures_count",0))
            .bind(maybe_field(tracker,"first_failure_at"))
            .bind(field(tracker,"created_at")?).bind(&imported_at)
            .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    sqlx::query("INSERT INTO webhook_delinquency_trackers(id,account_id,webhook_id,created_at,updated_at) SELECT w.id,w.account_id,w.id,w.created_at,w.updated_at FROM webhooks w LEFT JOIN webhook_delinquency_trackers t ON t.webhook_id=w.id WHERE w.account_id=? AND t.id IS NULL")
        .bind(account_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    for event in records(&archive, "events") {
        let eventable_id = field(event, "eventable_id")?;
        let eventable_type = field(event, "eventable_type")?;
        let card_id: String = match eventable_type {
            "Card" => eventable_id.to_owned(),
            "Comment" => sqlx::query_scalar("SELECT card_id FROM comments WHERE id=?")
                .bind(eventable_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e| e.to_string())?,
            _ => return Err(format!("Unsupported event type: {eventable_type}")),
        };
        let particulars = serde_json::to_string(event.get("particulars").unwrap_or(&json!({})))
            .map_err(|e| e.to_string())?;
        sqlx::query("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,particulars,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
            .bind(field(event,"id")?).bind(account_id).bind(field(event,"board_id")?).bind(card_id)
            .bind(field(event,"creator_id")?).bind(eventable_type).bind(eventable_id)
            .bind(field(event,"action")?).bind(particulars).bind(field(event,"created_at")?)
            .bind(&imported_at)
            .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for notification in records(&archive, "notifications") {
        sqlx::query("INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,unread_count,read_at,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
            .bind(field(notification,"id")?).bind(account_id).bind(field(notification,"user_id")?)
            .bind(field(notification,"creator_id")?).bind(field(notification,"card_id")?)
            .bind(field(notification,"source_id")?).bind(field(notification,"source_type")?)
            .bind(integer(notification,"unread_count",1)).bind(maybe_field(notification,"read_at"))
            .bind(field(notification,"created_at")?).bind(&imported_at)
            .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for bundle in records(&archive, "notification_bundles") {
        sqlx::query("INSERT INTO notification_bundles(id,account_id,user_id,starts_at,ends_at,status,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
            .bind(field(bundle,"id")?).bind(account_id).bind(field(bundle,"user_id")?)
            .bind(field(bundle,"starts_at")?).bind(field(bundle,"ends_at")?)
            .bind(bundle_status(bundle)?).bind(field(bundle,"created_at")?)
            .bind(&imported_at).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    for delivery in records(&archive, "webhook_deliveries") {
        let request = delivery
            .get("request")
            .filter(|value| !value.is_null())
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| e.to_string())?;
        let response = delivery
            .get("response")
            .filter(|value| !value.is_null())
            .map(serde_json::to_string)
            .transpose()
            .map_err(|e| e.to_string())?;
        sqlx::query("INSERT INTO webhook_deliveries(id,account_id,webhook_id,event_id,state,request,response,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)")
            .bind(field(delivery,"id")?).bind(account_id).bind(field(delivery,"webhook_id")?)
            .bind(field(delivery,"event_id")?).bind(field(delivery,"state")?)
            .bind(request).bind(response).bind(field(delivery,"created_at")?)
            .bind(&imported_at).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    let mut files = FileCleanup(Vec::new());
    let mut blob_sgids = HashMap::<String, String>::new();
    let mut seen_storage = HashSet::<String>::new();
    tokio::fs::create_dir_all(storage::storage_dir())
        .await
        .map_err(|e| e.to_string())?;
    for blob in records(&archive, "active_storage_blobs") {
        let blob_id = field(blob, "id")?;
        let old_key = field(blob, "key")?;
        if !seen_storage.insert(old_key.to_owned()) {
            return Err("Duplicate blob key".into());
        }
        let bytes = archive.storage.get(old_key);
        let checksum = field(blob, "checksum")?;
        let byte_size = integer(blob, "byte_size", -1);
        if let Some(bytes) = bytes {
            if bytes.len() as i64 != byte_size {
                return Err(format!("Storage size mismatch: {old_key}"));
            }
            let actual_checksum =
                base64::engine::general_purpose::STANDARD.encode(Md5::digest(bytes));
            if actual_checksum != checksum {
                return Err(format!("Storage checksum mismatch: {old_key}"));
            }
        }
        let new_key = random_token();
        let signed_id = app.sgid_signer.blob_signed_id(blob_id);
        let sgid = app.sgid_signer.blob(blob_id);
        let upload_hash = hash(&random_token());
        let metadata = serde_json::to_string(blob.get("metadata").unwrap_or(&json!({})))
            .map_err(|e| e.to_string())?;
        let path = storage::storage_dir().join(&new_key);
        if let Some(bytes) = bytes {
            tokio::fs::write(&path, bytes)
                .await
                .map_err(|e| e.to_string())?;
            files.0.push(path);
        }
        sqlx::query("INSERT INTO storage_blobs(id,account_id,storage_key,filename,content_type,byte_size,checksum,metadata,signed_id,attachable_sgid,upload_token_hash,upload_expires_at,uploaded_at,created_at) VALUES(?,?,?,?,?,?,?,?,?,?,?,?,?,?)")
            .bind(blob_id).bind(account_id).bind(new_key).bind(field(blob,"filename")?)
            .bind(field(blob,"content_type")?).bind(byte_size).bind(checksum)
            .bind(metadata)
            .bind(signed_id).bind(&sgid).bind(upload_hash).bind(now()).bind(bytes.map(|_| now()))
            .bind(field(blob,"created_at")?).execute(&mut *tx).await.map_err(|e| e.to_string())?;
        blob_sgids.insert(blob_id.to_owned(), sgid);
    }
    for variant in records(&archive, "active_storage_variant_records") {
        sqlx::query("INSERT INTO storage_variant_records(id,account_id,blob_id,variation_digest) VALUES(?,?,?,?)")
            .bind(field(variant,"id")?).bind(account_id).bind(field(variant,"blob_id")?)
            .bind(field(variant,"variation_digest")?)
            .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    if archive
        .storage
        .keys()
        .any(|key| !seen_storage.contains(key))
    {
        return Err("Storage file has no blob metadata".into());
    }
    let user_rows = sqlx::query("SELECT user_id,sgid FROM user_attachables WHERE account_id=?")
        .bind(account_id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    let user_sgids: HashMap<String, String> = user_rows
        .into_iter()
        .map(|row| (row.get("user_id"), row.get("sgid")))
        .collect();
    let gid = regex::Regex::new(r#"gid=["']gid://fizzy/(ActiveStorage::Blob|User)/([^"']+)["']"#)
        .map_err(|e| e.to_string())?;
    for rich in records(&archive, "action_text_rich_texts") {
        let body_is_null = rich.get("body").ok_or("Missing rich text body")?.is_null();
        let original = maybe_field(rich, "body").unwrap_or("");
        for capture in gid.captures_iter(original) {
            let valid = match &capture[1] {
                "ActiveStorage::Blob" => blob_sgids.contains_key(&capture[2]),
                "User" => user_sgids.contains_key(&capture[2]),
                _ => false,
            };
            if !valid {
                return Err("Rich text references unknown attachment".into());
            }
        }
        let body = gid
            .replace_all(original, |capture: &regex::Captures<'_>| {
                let sgid = if &capture[1] == "User" {
                    user_sgids
                        .get(&capture[2])
                        .expect("checked user")
                        .to_owned()
                } else {
                    blob_sgids
                        .get(&capture[2])
                        .expect("checked blob")
                        .to_owned()
                };
                format!("sgid=\"{sgid}\"")
            })
            .into_owned();
        let search = search_content(rich_text::plain_text(&body));
        let changed = match (field(rich,"record_type")?,field(rich,"name")?) {
            ("Card","description") => sqlx::query("UPDATE cards SET description=?,search_text=? WHERE id=? AND account_id=?")
                .bind(&body).bind(search).bind(field(rich,"record_id")?).bind(account_id)
                .execute(&mut *tx).await.map_err(|e| e.to_string())?,
            ("Comment","body") => sqlx::query("UPDATE comments SET body=?,search_text=? WHERE id=? AND card_id IN (SELECT id FROM cards WHERE account_id=?)")
                .bind(&body).bind(search).bind(field(rich,"record_id")?).bind(account_id)
                .execute(&mut *tx).await.map_err(|e| e.to_string())?,
            ("Board","description") => sqlx::query("UPDATE boards SET public_description=? WHERE id=? AND account_id=?")
                .bind(&body).bind(field(rich,"record_id")?).bind(account_id)
                .execute(&mut *tx).await.map_err(|e| e.to_string())?,
            _ => return Err("Unsupported rich text target".into()),
        };
        if changed.rows_affected() != 1 {
            return Err("Rich text references unknown record".into());
        }
        sqlx::query("INSERT INTO rich_text_records(id,account_id,record_type,record_id,name,created_at,updated_at,body_is_null) VALUES(?,?,?,?,?,?,?,?)")
            .bind(field(rich, "id")?)
            .bind(account_id)
            .bind(field(rich, "record_type")?)
            .bind(field(rich, "record_id")?)
            .bind(field(rich, "name")?)
            .bind(field(rich, "created_at")?)
            .bind(field(rich, "updated_at")?)
            .bind(body_is_null)
            .execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    let rich_targets: HashMap<&str, (&str, &str)> = records(&archive, "action_text_rich_texts")
        .iter()
        .filter_map(|rich| {
            Some((
                maybe_field(rich, "id")?,
                (
                    maybe_field(rich, "record_type")?,
                    maybe_field(rich, "record_id")?,
                ),
            ))
        })
        .collect();
    for attachment in records(&archive, "active_storage_attachments") {
        let (record_type, record_id) =
            if field(attachment, "record_type")? == "ActionText::RichText" {
                *rich_targets
                    .get(field(attachment, "record_id")?)
                    .ok_or("Attachment references unknown rich text")?
            } else {
                (
                    field(attachment, "record_type")?,
                    field(attachment, "record_id")?,
                )
            };
        sqlx::query("INSERT INTO storage_attachments(id,account_id,blob_id,record_type,record_id,name,created_at) VALUES(?,?,?,?,?,?,?)")
            .bind(field(attachment,"id")?).bind(account_id).bind(field(attachment,"blob_id")?)
            .bind(record_type).bind(record_id).bind(field(attachment,"name")?)
            .bind(field(attachment,"created_at")?).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    }
    // Card insert triggers touch their board, but Fizzy's bulk importer bypasses
    // callbacks and gives every imported board one fresh update timestamp.
    sqlx::query("UPDATE boards SET updated_at=? WHERE account_id=?")
        .bind(&imported_at)
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    // The source bulk importer bypasses Access's user touch callback.
    for user in records(&archive, "users") {
        sqlx::query("UPDATE users SET updated_at=? WHERE id=? AND account_id=?")
            .bind(field(user, "updated_at")?)
            .bind(field(user, "id")?)
            .bind(account_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| e.to_string())?;
    }
    let repair_count = if app.mysql {
        "UPDATE accounts SET cards_count=GREATEST(cards_count,COALESCE((SELECT MAX(number) FROM cards WHERE account_id=?),0)) WHERE id=?"
    } else {
        "UPDATE accounts SET cards_count=MAX(cards_count,COALESCE((SELECT MAX(number) FROM cards WHERE account_id=?),0)) WHERE id=?"
    };
    sqlx::query(repair_count)
        .bind(account_id)
        .bind(account_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    storage::reconcile_imported_storage_tx(&mut tx, account_id, app.mysql)
        .await
        .map_err(|error| error.1)?;
    let timestamp = now();
    let completed = sqlx::query("UPDATE imports SET status='completed',completed_at=?,updated_at=? WHERE id=? AND status='processing'")
        .bind(&timestamp).bind(&timestamp).bind(import_id).execute(&mut *tx).await.map_err(|e| e.to_string())?;
    if completed.rows_affected() != 1 {
        return Err("Import claim was lost".into());
    }
    tx.commit().await.map_err(|e| e.to_string())?;
    files.0.clear();
    Ok(())
}

async fn restore(app: &App, import_id: &str, account_id: &str) -> ImportResult<()> {
    let path = import_path(import_id);
    let archive = tokio::task::spawn_blocking(move || load_archive(&path))
        .await
        .map_err(|e| e.to_string())??;
    apply_archive(app, import_id, account_id, archive).await
}

pub(super) async fn worker(app: Arc<App>) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    let mut ticks = 0_u64;
    loop {
        let timer_tick = tokio::select! {
            _ = interval.tick() => true,
            _ = app.import_notify.notified() => false,
        };
        if timer_tick {
            ticks += 1;
        }
        if timer_tick && (ticks == 1 || ticks.is_multiple_of(60)) {
            let stale = (Utc::now() - chrono::Duration::minutes(30))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            if let Err(error) = sqlx::query(
                "UPDATE imports SET status='pending' WHERE status='processing' AND updated_at<?",
            )
            .bind(stale)
            .execute(&app.db)
            .await
            {
                eprintln!("import recovery failed: {error}");
            }
        }
        let pending = sqlx::query(
            "SELECT id,account_id FROM imports WHERE status='pending' ORDER BY created_at LIMIT 1",
        )
        .fetch_optional(&app.db)
        .await;
        let Some(row) = (match pending {
            Ok(row) => row,
            Err(error) => {
                eprintln!("import queue read failed: {error}");
                continue;
            }
        }) else {
            continue;
        };
        let import_id: String = row.get("id");
        let account_id: String = row.get("account_id");
        let claim = sqlx::query(
            "UPDATE imports SET status='processing',updated_at=? WHERE id=? AND status='pending'",
        )
        .bind(now())
        .bind(&import_id)
        .execute(&app.db)
        .await;
        if !claim.is_ok_and(|result| result.rows_affected() == 1) {
            continue;
        }
        match restore(&app, &import_id, &account_id).await {
            Ok(()) => {
                if let Err(error) = import_mail::send(&app, &import_id, true, None).await {
                    eprintln!("import completion email failed: {}", error.1);
                }
            }
            Err(error) => {
                eprintln!("import {import_id} failed: {error}");
                let reason = if error.contains("UNIQUE constraint")
                    || error.contains("PRIMARY KEY")
                    || error.contains("1062 (23000)")
                {
                    "conflict"
                } else {
                    "invalid_export"
                };
                let recorded = sqlx::query(
                    "UPDATE imports SET status='failed',failure_reason=?,updated_at=? WHERE id=?",
                )
                .bind(reason)
                .bind(now())
                .bind(&import_id)
                .execute(&app.db)
                .await;
                if recorded.is_ok_and(|result| result.rows_affected() == 1) {
                    if let Err(error) =
                        import_mail::send(&app, &import_id, false, Some(reason)).await
                    {
                        eprintln!("import failure email failed: {}", error.1);
                    }
                }
            }
        }
        app.import_notify.notify_one();
    }
}

pub(super) async fn cleanup(app: &App) -> std::result::Result<(), sqlx::Error> {
    let completed = (Utc::now() - chrono::Duration::hours(24))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let failed = (Utc::now() - chrono::Duration::days(7))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let rows = sqlx::query("SELECT id,account_id,status FROM imports WHERE (status='completed' AND completed_at<?) OR (status='failed' AND created_at<?)")
        .bind(completed).bind(failed).fetch_all(&app.db).await?;
    for row in rows {
        let import_id: String = row.get("id");
        let account_id: String = row.get("account_id");
        let status: String = row.get("status");
        let _ = tokio::fs::remove_file(import_path(&import_id)).await;
        let mut tx = app.db.begin().await?;
        sqlx::query("DELETE FROM imports WHERE id=?")
            .bind(&import_id)
            .execute(&mut *tx)
            .await?;
        if status == "failed" {
            sqlx::query("DELETE FROM users WHERE account_id=?")
                .bind(&account_id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM accounts WHERE id=?")
                .bind(&account_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
    }
    Ok(())
}
