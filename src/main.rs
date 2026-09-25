use axum::{
    Json, Router, ServiceExt,
    body::{Body, Bytes},
    extract::{ConnectInfo, OriginalUri, Path, Query, RawQuery, Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Datelike, Utc};
use rand::Rng;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    QueryBuilder, Row, Sqlite, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteRow},
};
use std::{
    collections::{HashMap, HashSet},
    env,
    net::{IpAddr, SocketAddr},
    process::Stdio,
    sync::Arc,
    time::Duration,
};
use tokio::io::AsyncWriteExt;
use tokio::net::lookup_host;
use tokio::process::Command;
use tower::{Layer, util::MapRequestLayer};
mod access_tokens_page;
mod account_cancellation;
mod account_settings;
mod account_transfer;
mod assignment_page;
mod board_page;
mod board_settings;
mod board_views;
mod card_board_page;
mod card_column_page;
mod card_controls;
mod card_page;
mod cards_page;
mod comment_edit_page;
mod events_page;
mod exports;
mod imports;
mod join_code_page;
mod my_menu;
mod notification_mail;
mod passkeys;
mod push_delivery;
mod push_subscriptions;
mod pwa;
mod reaction_page;
mod rich_text;
mod session_transfer;
mod signup_page;
mod signup_seed;
mod storage;
mod tagging_page;
mod unsubscribes;
mod user_page;
mod webhook_page;

struct App {
    db: SqlitePool,
    write_lock: tokio::sync::Mutex<()>,
    auth_lock: tokio::sync::Mutex<()>,
    export_notify: tokio::sync::Notify,
    import_notify: tokio::sync::Notify,
}
type Shared = State<Arc<App>>;
type Result<T> = std::result::Result<T, ApiError>;

#[derive(Debug)]
struct ApiError(StatusCode, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.0 == StatusCode::FOUND {
            return (StatusCode::FOUND, [(header::LOCATION, self.1)]).into_response();
        }
        if self.0 == StatusCode::UNAUTHORIZED && self.1 == "HTTP Token: Access denied." {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Token realm=\"Application\"")],
                "HTTP Token: Access denied.\n",
            )
                .into_response();
        }
        (self.0, Json(json!({"message": self.1}))).into_response()
    }
}
fn err(status: StatusCode, message: &str) -> ApiError {
    ApiError(status, message.into())
}
fn token_denied() -> ApiError {
    err(StatusCode::UNAUTHORIZED, "HTTP Token: Access denied.")
}
fn rewrite_json_uri<B>(mut request: axum::http::Request<B>) -> axum::http::Request<B> {
    // This runs outside the router so collection routes such as /boards.json
    // can match /boards. Explicit client configuration paths have their own routes.
    request.headers_mut().remove("x-rizzy-original-path");
    let original = request.uri().clone();
    let path = original.path();
    if !path.ends_with(".json")
        || path.contains("/client_configurations/")
        || path == "/manifest.json"
    {
        return request;
    }
    let stripped = &path[..path.len() - ".json".len()];
    if stripped.is_empty() || stripped == "/" {
        return request;
    }
    let path_and_query = match original.query() {
        Some(query) => format!("{stripped}?{query}"),
        None => stripped.to_owned(),
    };
    let Ok(path_and_query) = path_and_query.parse() else {
        return request;
    };
    let mut parts = original.clone().into_parts();
    parts.path_and_query = Some(path_and_query);
    let Ok(rewritten) = Uri::from_parts(parts) else {
        return request;
    };
    if let Ok(value) = HeaderValue::from_str(path) {
        request.headers_mut().insert("x-rizzy-original-path", value);
    }
    // A format suffix takes precedence over a conflicting Accept header.
    request
        .headers_mut()
        .insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    *request.uri_mut() = rewritten;
    request
}
async fn mark_request_format(mut request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let json = path.ends_with(".json")
        || (path.contains("/exports/") && path.ends_with("/download"))
        || request
            .headers()
            .get(header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("application/json"));
    let write = !matches!(request.method().as_str(), "GET" | "HEAD");
    request.headers_mut().remove("x-rizzy-json-request");
    request.headers_mut().remove("x-rizzy-write-request");
    if json {
        request
            .headers_mut()
            .insert("x-rizzy-json-request", HeaderValue::from_static("1"));
    }
    if write {
        request
            .headers_mut()
            .insert("x-rizzy-write-request", HeaderValue::from_static("1"));
    }
    next.run(request).await
}
async fn register_pwa_on_pages(State(app): Shared, request: Request, next: Next) -> Response {
    let session_cookie = cookie_value(request.headers(), "session_token").is_some();
    let headers = if session_cookie {
        Some(request.headers().clone())
    } else {
        None
    };
    let response = next.run(request).await;
    if !response.status().is_success()
        || !response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("text/html"))
    {
        return response;
    }
    let Some(headers) = headers else {
        return response;
    };
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, usize::MAX).await else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "Could not render page").into_response();
    };
    let leading = bytes
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(bytes.len());
    if !bytes
        .get(leading..leading.saturating_add("<!doctype html".len()))
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(b"<!doctype html"))
    {
        return Response::from_parts(parts, Body::from(bytes));
    }
    if identity_auth(&app, &headers).await.is_err() {
        return Response::from_parts(parts, Body::from(bytes));
    }
    let Ok(mut html) = String::from_utf8(bytes.to_vec()) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    if let Some(head) = html.find("</head>") {
        html.insert_str(head, "<link rel=\"manifest\" href=\"/manifest.json\">");
    }
    let script_position = html
        .find("</body>")
        .or_else(|| html.find("</html>"))
        .unwrap_or(html.len());
    html.insert_str(script_position, include_str!("../assets/pwa_register.html"));
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(html))
}
fn db_err(e: sqlx::Error) -> ApiError {
    eprintln!("database error: {e}");
    err(StatusCode::INTERNAL_SERVER_ERROR, "Database error")
}
fn bad(message: &str) -> ApiError {
    err(StatusCode::UNPROCESSABLE_ENTITY, message)
}
fn missing() -> ApiError {
    err(StatusCode::NOT_FOUND, "Not found")
}
fn now() -> String {
    Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
fn id() -> String {
    let mut n = uuid::Uuid::now_v7().as_u128();
    let mut chars = [b'0'; 25];
    for p in (0..25).rev() {
        let d = (n % 36) as u8;
        chars[p] = if d < 10 { b'0' + d } else { b'a' + d - 10 };
        n /= 36;
    }
    String::from_utf8(chars.to_vec()).expect("base36")
}
fn hash(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}
fn random_token() -> String {
    hex::encode(rand::random::<[u8; 32]>())
}
fn join_code() -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut rng = rand::rng();
    let chars: String = (0..12)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect();
    format!("{}-{}-{}", &chars[..4], &chars[4..8], &chars[8..])
}
async fn insert_join_code_tx(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    account_id: &str,
) -> std::result::Result<(), sqlx::Error> {
    loop {
        let timestamp = now();
        let inserted = sqlx::query("INSERT OR IGNORE INTO account_join_codes(id,account_id,code,created_at,updated_at) VALUES(?,?,?,?,?)")
            .bind(id()).bind(account_id).bind(join_code()).bind(&timestamp).bind(&timestamp)
            .execute(&mut **tx).await?;
        if inserted.rows_affected() == 1 {
            return Ok(());
        }
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM account_join_codes WHERE account_id=?)",
        )
        .bind(account_id)
        .fetch_one(&mut **tx)
        .await?;
        if exists != 0 {
            return Ok(());
        }
    }
}
fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|line| line.split(';'))
        .filter_map(|part| part.trim().split_once('='))
        .find(|(key, _)| *key == name)
        .map(|(_, value)| value.to_string())
}
fn set_cookie(headers: &mut HeaderMap, name: &str, value: &str, max_age: i64, secure: bool) {
    let secure_part = if secure { "; Secure" } else { "" };
    let cookie =
        format!("{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age}{secure_part}");
    if let Ok(v) = HeaderValue::from_str(&cookie) {
        headers.append(header::SET_COOKIE, v);
    }
}
fn join_return_to(headers: &HeaderMap) -> Option<String> {
    let value = cookie_value(headers, "join_return_to")?;
    let account = value
        .strip_prefix('/')?
        .strip_suffix("/users/verifications/new")?;
    if !account.is_empty() && account.bytes().all(|byte| byte.is_ascii_digit()) {
        Some(value)
    } else {
        None
    }
}
fn secure_request(headers: &HeaderMap) -> bool {
    headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        == Some("https")
}
fn base(headers: &HeaderMap) -> String {
    let host = headers
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("localhost:3007");
    let scheme = headers
        .get("x-forwarded-proto")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("http");
    format!("{scheme}://{host}")
}
fn nested<'a>(body: &'a Value, key: &str) -> &'a Value {
    body.get(key).unwrap_or(body)
}
fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn required(v: &Value, key: &str) -> Result<String> {
    string(v, key)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| bad(&format!("{key} is required")))
}
fn timestamp_param(v: &Value, key: &str) -> Result<Option<String>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => chrono::DateTime::parse_from_rfc3339(raw)
            .map(|value| {
                Some(
                    value
                        .with_timezone(&Utc)
                        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
                )
            })
            .map_err(|_| bad(&format!("Invalid {key}"))),
        Some(_) => Err(bad(&format!("Invalid {key}"))),
    }
}
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn rich_html(s: &str) -> String {
    rich_text::html(s)
}
fn search_content(mut text: String) -> String {
    const LIMIT: usize = 32 * 1024;
    if text.len() > LIMIT {
        let mut boundary = LIMIT;
        while !text.is_char_boundary(boundary) {
            boundary -= 1;
        }
        text.truncate(boundary);
    }
    text
}
fn wrap_html(s: &str) -> String {
    format!("<div class=\"action-text-content\">\n  {s}\n</div>\n")
}
fn system_plain_text(s: &str) -> String {
    s.replace("<strong>", "")
        .replace("</strong>", "")
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
}
fn created(location: String, body: Value) -> Response {
    (
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(body),
    )
        .into_response()
}
fn page_window(page: i64) -> (i64, i64) {
    let page = page.max(1);
    let sizes = [15_i64, 30, 50, 100];
    let prior = (1..page.min(5))
        .map(|p| sizes[(p - 1) as usize])
        .sum::<i64>();
    let offset = prior + (page - 5).max(0) * 100;
    (offset, sizes[(page - 1).min(3) as usize])
}
fn paginated(items: Value, total: i64, page: i64, uri: &Uri, headers: &HeaderMap) -> Response {
    let mut response = Json(items).into_response();
    response.headers_mut().insert(
        "x-total-count",
        HeaderValue::from_str(&total.to_string()).expect("count header"),
    );
    let (offset, limit) = page_window(page);
    if offset + limit < total {
        let mut pairs: Vec<(String, String)> = uri
            .query()
            .map(|q| {
                url::form_urlencoded::parse(q.as_bytes())
                    .filter(|(k, _)| k != "page")
                    .map(|(k, v)| (k.into_owned(), v.into_owned()))
                    .collect()
            })
            .unwrap_or_default();
        pairs.push(("page".into(), (page + 1).to_string()));
        let query = url::form_urlencoded::Serializer::new(String::new())
            .extend_pairs(pairs)
            .finish();
        let original_path = headers
            .get("x-rizzy-original-path")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_else(|| uri.path());
        let link = format!(
            "<{}{path}?{query}>; rel=\"next\"",
            base(headers),
            path = original_path
        );
        if let Ok(value) = HeaderValue::from_str(&link) {
            response.headers_mut().insert(header::LINK, value);
        }
    }
    response
}

#[derive(Clone)]
struct Principal {
    account_id: String,
    user_id: String,
    role: String,
    permission: String,
    slug: i64,
}
impl Principal {
    fn write(&self) -> Result<()> {
        if self.permission == "write" {
            Ok(())
        } else {
            Err(err(StatusCode::FORBIDDEN, "Write permission required"))
        }
    }
    fn admin(&self, creator_id: &str) -> Result<()> {
        if self.user_id == creator_id || self.role == "owner" || self.role == "admin" {
            Ok(())
        } else {
            Err(err(StatusCode::FORBIDDEN, "Board administrator required"))
        }
    }
    fn account_admin(&self) -> Result<()> {
        if self.role == "owner" || self.role == "admin" {
            Ok(())
        } else {
            Err(err(StatusCode::FORBIDDEN, "Account administrator required"))
        }
    }
}
async fn principal(app: &App, headers: &HeaderMap, slug: &str) -> Result<Principal> {
    let slug: i64 = slug.parse().map_err(|_| missing())?;
    let (identity_id, permission) = identity_auth(app, headers).await?;
    let r = sqlx::query("SELECT a.id account_id,u.id user_id,u.role,EXISTS(SELECT 1 FROM account_cancellations c WHERE c.account_id=a.id) cancelled FROM users u JOIN accounts a ON a.id=u.account_id WHERE u.identity_id=? AND a.external_account_id=? AND u.active=1")
        .bind(identity_id).bind(slug).fetch_optional(&app.db).await.map_err(db_err)?
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED,"Invalid token or account"))?;
    if r.get::<i64, _>("cancelled") != 0 {
        return Err(if headers.contains_key("x-rizzy-json-request") {
            err(StatusCode::FORBIDDEN, "Account access required")
        } else {
            ApiError(StatusCode::FOUND, "/session/menu".into())
        });
    }
    Ok(Principal {
        account_id: r.get("account_id"),
        user_id: r.get("user_id"),
        role: r.get("role"),
        permission,
        slug,
    })
}
async fn board(app: &App, p: &Principal, id: &str) -> Result<SqliteRow> {
    sqlx::query("SELECT b.* FROM boards b WHERE b.id=? AND b.account_id=? AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?)")
        .bind(id.trim_end_matches(".json")).bind(&p.account_id).bind(&p.user_id).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)
}
async fn card(app: &App, p: &Principal, number: &str) -> Result<SqliteRow> {
    let n: i64 = number
        .trim_end_matches(".json")
        .parse()
        .map_err(|_| missing())?;
    sqlx::query("SELECT c.* FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.account_id=? AND c.number=? AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?)")
        .bind(&p.account_id).bind(n).bind(&p.user_id).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)
}
async fn user_json(app: &App, p: &Principal, id: &str, host: &str) -> Result<Value> {
    let r = sqlx::query("SELECT u.*,i.email_address FROM users u JOIN identities i ON i.id=u.identity_id WHERE u.id=? AND u.account_id=?")
        .bind(id).bind(&p.account_id).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    Ok(user_json_row(p, &r, host))
}
fn user_json_row(p: &Principal, r: &SqliteRow, host: &str) -> Value {
    let id: String = r.get("id");
    let role: String = r.get("role");
    let email: Option<String> = if role == "system" {
        None
    } else {
        Some(r.get("email_address"))
    };
    json!({"id":id,"name":r.get::<String,_>("name"),"role":role,"active":r.get::<i64,_>("active")!=0,"email_address":email,"created_at":r.get::<String,_>("created_at"),"url":format!("{host}/{}/users/{id}",p.slug),"avatar_url":format!("{host}/{}/users/{id}/avatar",p.slug)})
}
async fn actor_name(app: &App, p: &Principal) -> Result<String> {
    sqlx::query_scalar("SELECT name FROM users WHERE id=? AND account_id=?")
        .bind(&p.user_id)
        .bind(&p.account_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)
}
async fn system_user(app: &App, p: &Principal) -> Result<String> {
    if let Some(id) =
        sqlx::query_scalar::<_, String>("SELECT id FROM users WHERE account_id=? AND role='system'")
            .bind(&p.account_id)
            .fetch_optional(&app.db)
            .await
            .map_err(db_err)?
    {
        return Ok(id);
    }
    let identity_id = id();
    let user_id = id();
    let timestamp = now();
    let email = format!("system+{}@rizzy.invalid", p.account_id);
    sqlx::query("INSERT OR IGNORE INTO identities(id,email_address,created_at) VALUES(?,?,?)")
        .bind(&identity_id)
        .bind(&email)
        .bind(&timestamp)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    let identity_id: String = sqlx::query_scalar("SELECT id FROM identities WHERE email_address=?")
        .bind(&email)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    sqlx::query("INSERT OR IGNORE INTO users(id,account_id,identity_id,name,role,created_at) VALUES(?,?,?,'System','system',?)")
        .bind(&user_id).bind(&p.account_id).bind(&identity_id).bind(&timestamp).execute(&app.db).await.map_err(db_err)?;
    sqlx::query_scalar("SELECT id FROM users WHERE account_id=? AND role='system'")
        .bind(&p.account_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)
}
async fn system_comment(app: &App, p: &Principal, card_id: &str, body: &str) -> Result<()> {
    let system = system_user(app, p).await?;
    let timestamp = now();
    let (plain, _, _) = storage::render_rich_text(app, &p.account_id, p.slug, body).await?;
    let content = search_content(plain);
    sqlx::query("INSERT INTO comments(id,card_id,creator_id,body,search_text,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
        .bind(id()).bind(card_id).bind(system).bind(body).bind(content).bind(&timestamp).bind(&timestamp).execute(&app.db).await.map_err(db_err)?;
    Ok(())
}
async fn record_event(
    app: &App,
    p: &Principal,
    board_id: &str,
    card_id: &str,
    eventable_type: &str,
    eventable_id: &str,
    action: &str,
    particulars: Value,
) -> Result<()> {
    let timestamp = now();
    let event_id = id();
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,particulars,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)")
        .bind(&event_id).bind(&p.account_id).bind(board_id).bind(card_id).bind(&p.user_id).bind(eventable_type)
        .bind(eventable_id).bind(action).bind(particulars.to_string()).bind(&timestamp)
        .execute(&mut *tx).await.map_err(db_err)?;
    queue_webhooks(
        &mut tx,
        &p.account_id,
        board_id,
        &event_id,
        action,
        &timestamp,
    )
    .await?;
    queue_notifications(
        &mut tx,
        p,
        board_id,
        card_id,
        &event_id,
        eventable_type,
        action,
        &particulars,
        &timestamp,
    )
    .await?;
    sqlx::query("UPDATE cards SET last_active_at=?,updated_at=? WHERE id=?")
        .bind(&timestamp)
        .bind(&timestamp)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}
async fn record_activity_spike(app: &App, account_id: &str, card_id: &str) -> Result<()> {
    let timestamp = now();
    sqlx::query("INSERT INTO card_activity_spikes(card_id,account_id,created_at,updated_at) VALUES(?,?,?,?) ON CONFLICT(card_id) DO UPDATE SET updated_at=excluded.updated_at")
        .bind(card_id).bind(account_id).bind(&timestamp).bind(&timestamp)
        .execute(&app.db).await.map_err(db_err)?;
    Ok(())
}
async fn queue_webhooks(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    account_id: &str,
    board_id: &str,
    event_id: &str,
    action: &str,
    timestamp: &str,
) -> Result<()> {
    let webhooks:Vec<String>=sqlx::query_scalar("SELECT w.id FROM webhooks w WHERE w.account_id=? AND w.board_id=? AND w.active=1 AND EXISTS(SELECT 1 FROM json_each(w.subscribed_actions) WHERE value=?)")
        .bind(account_id).bind(board_id).bind(action).fetch_all(&mut **tx).await.map_err(db_err)?;
    for webhook_id in webhooks {
        sqlx::query("INSERT INTO webhook_deliveries(id,account_id,webhook_id,event_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(id()).bind(account_id).bind(webhook_id).bind(event_id).bind(timestamp).bind(timestamp)
            .execute(&mut **tx).await.map_err(db_err)?;
    }
    Ok(())
}

async fn referenced_users(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    account_id: &str,
    content: &str,
) -> Result<Vec<String>> {
    let mut users = HashSet::new();
    for sgid in rich_text::attachment_sgids(content) {
        let user_id: Option<String> = if let Some(legacy_id) =
            sgid.strip_prefix("gid://fizzy/User/")
        {
            sqlx::query_scalar("SELECT id FROM users WHERE id=? AND account_id=?")
                .bind(legacy_id)
                .bind(account_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(db_err)?
        } else {
            sqlx::query_scalar("SELECT user_id FROM user_attachables WHERE sgid=? AND account_id=?")
                .bind(&sgid)
                .bind(account_id)
                .fetch_optional(&mut **tx)
                .await
                .map_err(db_err)?
        };
        if let Some(user_id) = user_id {
            users.insert(user_id);
        }
    }
    Ok(users.into_iter().collect())
}

async fn queue_notifications(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    p: &Principal,
    board_id: &str,
    card_id: &str,
    event_id: &str,
    eventable_type: &str,
    action: &str,
    particulars: &Value,
    timestamp: &str,
) -> Result<()> {
    if p.role == "system" {
        return Ok(());
    }
    let mention_content = if action == "card_published" {
        sqlx::query_scalar::<_, String>("SELECT description FROM cards WHERE id=?")
            .bind(card_id)
            .fetch_optional(&mut **tx)
            .await
            .map_err(db_err)?
    } else if eventable_type == "Comment" {
        sqlx::query_scalar::<_, String>(
            "SELECT body FROM comments WHERE id=(SELECT eventable_id FROM events WHERE id=?)",
        )
        .bind(event_id)
        .fetch_optional(&mut **tx)
        .await
        .map_err(db_err)?
    } else {
        None
    };
    let mentioned: HashSet<String> = if let Some(content) = mention_content {
        referenced_users(tx, &p.account_id, &content)
            .await?
            .into_iter()
            .collect()
    } else {
        HashSet::new()
    };
    let recipients: Vec<String> = if action == "card_assigned" {
        particulars["assignee_ids"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|value| value.as_str().map(str::to_owned))
            .collect()
    } else if eventable_type == "Comment" {
        sqlx::query_scalar("SELECT w.user_id FROM watches w JOIN users u ON u.id=w.user_id JOIN accesses a ON a.user_id=u.id AND a.board_id=? WHERE w.card_id=? AND w.watching=1 AND u.active=1")
            .bind(board_id).bind(card_id).fetch_all(&mut **tx).await.map_err(db_err)?
    } else {
        sqlx::query_scalar("SELECT a.user_id FROM accesses a JOIN users u ON u.id=a.user_id WHERE a.board_id=? AND a.involvement='watching' AND u.active=1")
            .bind(board_id).fetch_all(&mut **tx).await.map_err(db_err)?
    };
    for recipient in unique_ids(recipients)
        .into_iter()
        .filter(|user| user != &p.user_id && !mentioned.contains(user))
    {
        let inserted = sqlx::query("INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,unread_count,created_at,updated_at) SELECT ?,?,?,?,?,?,'Event',1,?,? WHERE EXISTS(SELECT 1 FROM accesses a JOIN users u ON u.id=a.user_id WHERE a.board_id=? AND a.user_id=? AND u.active=1) ON CONFLICT(user_id,card_id) DO UPDATE SET source_id=excluded.source_id,creator_id=excluded.creator_id,source_type='Event',read_at=NULL,unread_count=notifications.unread_count+1,updated_at=excluded.updated_at")
            .bind(id()).bind(&p.account_id).bind(&recipient).bind(&p.user_id).bind(card_id).bind(event_id)
            .bind(timestamp).bind(timestamp).bind(board_id).bind(&recipient)
            .execute(&mut **tx).await.map_err(db_err)?;
        if inserted.rows_affected() != 0 {
            notification_mail::queue(tx, &recipient, timestamp).await?;
            push_delivery::queue(tx, &recipient, card_id).await?;
        }
    }
    Ok(())
}

async fn create_new_mentions(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    p: &Principal,
    board_id: &str,
    card_id: &str,
    source_type: &str,
    source_id: &str,
    content: &str,
) -> Result<()> {
    for user_id in referenced_users(tx, &p.account_id, content).await? {
        let accessible: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users u JOIN accesses a ON a.user_id=u.id WHERE u.id=? AND u.account_id=? AND a.board_id=?")
            .bind(&user_id).bind(&p.account_id).bind(board_id)
            .fetch_one(&mut **tx).await.map_err(db_err)?;
        if accessible == 0 {
            continue;
        }
        let mention_id = id();
        let timestamp = now();
        let inserted = sqlx::query("INSERT INTO mentions(id,account_id,mentionee_id,mentioner_id,source_type,source_id,created_at,updated_at) SELECT ?,?,?,?,?,?,?,? WHERE NOT EXISTS(SELECT 1 FROM mentions WHERE mentionee_id=? AND mentioner_id=? AND source_type=? AND source_id=?)")
            .bind(&mention_id).bind(&p.account_id).bind(&user_id).bind(&p.user_id).bind(source_type).bind(source_id)
            .bind(&timestamp).bind(&timestamp).bind(&user_id).bind(&p.user_id).bind(source_type).bind(source_id)
            .execute(&mut **tx).await.map_err(db_err)?;
        if inserted.rows_affected() == 0 {
            continue;
        }
        sqlx::query("INSERT INTO watches(card_id,user_id,watching) VALUES(?,?,1) ON CONFLICT(card_id,user_id) DO UPDATE SET watching=1")
            .bind(card_id).bind(&user_id).execute(&mut **tx).await.map_err(db_err)?;
        if user_id != p.user_id && p.role != "system" {
            sqlx::query("INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,unread_count,created_at,updated_at) VALUES(?,?,?,?,?,?,'Mention',1,?,?) ON CONFLICT(user_id,card_id) DO UPDATE SET source_id=excluded.source_id,creator_id=excluded.creator_id,source_type='Mention',read_at=NULL,unread_count=notifications.unread_count+1,updated_at=excluded.updated_at")
                .bind(id()).bind(&p.account_id).bind(&user_id).bind(&p.user_id).bind(card_id).bind(&mention_id)
                .bind(&timestamp).bind(&timestamp).execute(&mut **tx).await.map_err(db_err)?;
            notification_mail::queue(tx, &user_id, &timestamp).await?;
            push_delivery::queue(tx, &user_id, card_id).await?;
        }
    }
    Ok(())
}

fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            let [a, b, c, _] = ip.octets();
            !matches!(a, 0 | 10 | 127 | 224..=255)
                && !(a == 100 && (64..=127).contains(&b))
                && !(a == 169 && b == 254)
                && !(a == 172 && (16..=31).contains(&b))
                && !(a == 192 && (b == 0 || b == 168 || (b == 88 && c == 99)))
                && !(a == 198 && (b == 18 || b == 19 || (b == 51 && c == 100)))
                && !(a == 203 && b == 0 && c == 113)
        }
        IpAddr::V6(ip) => {
            let seg = ip.segments();
            (seg[0] & 0xe000) == 0x2000 && !(seg[0] == 0x2001 && seg[1] == 0x0db8)
        }
    }
}

fn hmac_sha256(secret: &str, body: &str) -> String {
    let mut key = [0u8; 64];
    let bytes = secret.as_bytes();
    if bytes.len() > 64 {
        key[..32].copy_from_slice(&Sha256::digest(bytes));
    } else {
        key[..bytes.len()].copy_from_slice(bytes);
    }
    let mut inner = key;
    let mut outer = key;
    for byte in &mut inner {
        *byte ^= 0x36;
    }
    for byte in &mut outer {
        *byte ^= 0x5c;
    }
    let inner_hash = Sha256::digest([inner.as_slice(), body.as_bytes()].concat());
    hex::encode(Sha256::digest(
        [outer.as_slice(), inner_hash.as_slice()].concat(),
    ))
}

#[cfg(test)]
mod webhook_security_tests {
    use super::{hmac_sha256, public_ip};

    #[test]
    fn signature_matches_hmac_sha256_reference_vector() {
        assert_eq!(
            hmac_sha256("Jefe", "what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn private_and_reserved_destinations_are_blocked() {
        for address in [
            "127.0.0.1",
            "10.0.0.1",
            "169.254.169.254",
            "192.168.1.1",
            "198.51.100.1",
            "::1",
            "fc00::1",
            "2001:db8::1",
        ] {
            assert!(!public_ip(address.parse().unwrap()), "{address}");
        }
        for address in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(public_ip(address.parse().unwrap()), "{address}");
        }
    }
}

async fn webhook_payload(app: &App, row: &SqliteRow, host: &str) -> Result<String> {
    let account_id: String = row.get("account_id");
    let slug: i64 = row.get("external_account_id");
    let creator_id: String = row.get("creator_id");
    let principal = Principal {
        account_id,
        user_id: creator_id,
        slug,
        role: "owner".into(),
        permission: "write".into(),
    };
    let event = sqlx::query("SELECT * FROM events WHERE id=?")
        .bind(row.get::<String, _>("event_id"))
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let item = activity_items(app, &principal, &[event], host)
        .await?
        .into_iter()
        .next()
        .ok_or_else(missing)?;
    Ok(
        json!({"id":item["id"],"action":item["action"],"created_at":item["created_at"],
        "eventable":item["eventable"],"board":item["board"],"creator":item["creator"]})
        .to_string(),
    )
}

async fn send_webhook(url: &str, secret: &str, timestamp: &str, body: &str) -> Value {
    let Ok(uri) = url::Url::parse(url) else {
        return json!({"error":"destination_unreachable"});
    };
    if !matches!(uri.scheme(), "http" | "https") {
        return json!({"error":"destination_unreachable"});
    }
    let Some(host) = uri.host_str() else {
        return json!({"error":"dns_lookup_failed"});
    };
    let Some(port) = uri.port_or_known_default() else {
        return json!({"error":"destination_unreachable"});
    };
    let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        match tokio::time::timeout(Duration::from_secs(7), lookup_host((host, port))).await {
            Ok(Ok(found)) => found.collect::<Vec<_>>(),
            _ => return json!({"error":"dns_lookup_failed"}),
        }
    };
    // Refuse the destination if DNS returns even one private or reserved address.
    if addresses.is_empty() {
        return json!({"error":"dns_lookup_failed"});
    }
    if addresses.iter().any(|address| !public_ip(address.ip())) {
        return json!({"error":"private_uri"});
    }
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(7))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve_to_addrs(host, &addresses)
        .build()
    {
        Ok(client) => client,
        Err(_) => return json!({"error":"destination_unreachable"}),
    };
    let result = client
        .post(uri)
        .header("User-Agent", "fizzy/1.0.0 Webhook")
        .header("Content-Type", "application/json")
        .header("X-Webhook-Timestamp", timestamp)
        .header("X-Webhook-Signature", hmac_sha256(secret, body))
        .body(body.to_owned())
        .send()
        .await;
    let Ok(mut response) = result else {
        let error = result.unwrap_err();
        return if error.is_timeout() {
            json!({"error":"connection_timeout"})
        } else if error.is_connect() {
            json!({"error":"destination_unreachable"})
        } else {
            json!({"error":"destination_unreachable"})
        };
    };
    let code = response.status().as_u16();
    let mut bytes = 0usize;
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                bytes += chunk.len();
                if bytes > 100_000 {
                    return json!({"error":"response_too_large"});
                }
            }
            Ok(None) => break,
            Err(error) => {
                return if error.is_timeout() {
                    json!({"error":"connection_timeout"})
                } else {
                    json!({"error":"destination_unreachable"})
                };
            }
        }
    }
    json!({"code":code})
}

async fn process_webhook_delivery(app: &App, delivery_id: &str, host: &str) -> Result<()> {
    let row = sqlx::query("SELECT d.*,w.url,w.signing_secret,e.creator_id,e.created_at event_created_at,a.external_account_id FROM webhook_deliveries d JOIN webhooks w ON w.id=d.webhook_id JOIN events e ON e.id=d.event_id JOIN accounts a ON a.id=d.account_id WHERE d.id=?")
        .bind(delivery_id).fetch_one(&app.db).await.map_err(db_err)?;
    let timestamp: String = row.get("event_created_at");
    let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp)
        .map(|t| {
            t.to_utc()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .unwrap_or(timestamp);
    let request = json!({"headers":{"User-Agent":"fizzy/1.0.0 Webhook","Content-Type":"application/json","X-Webhook-Timestamp":timestamp}});
    let payload = webhook_payload(app, &row, host).await?;
    let response = send_webhook(
        &row.get::<String, _>("url"),
        &row.get::<String, _>("signing_secret"),
        &timestamp,
        &payload,
    )
    .await;
    let completed_at = now();
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("UPDATE webhook_deliveries SET state='completed',request=?,response=?,updated_at=? WHERE id=?")
        .bind(request.to_string()).bind(response.to_string()).bind(&completed_at).bind(delivery_id)
        .execute(&mut *tx).await.map_err(db_err)?;
    let webhook_id: String = row.get("webhook_id");
    let succeeded = response["code"]
        .as_u64()
        .is_some_and(|code| (200..300).contains(&code));
    if succeeded {
        sqlx::query(
            "UPDATE webhooks SET consecutive_failures_count=0,first_failure_at=NULL WHERE id=?",
        )
        .bind(&webhook_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    } else {
        let threshold = (Utc::now() - chrono::Duration::hours(1))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        sqlx::query("UPDATE webhooks SET active=CASE WHEN consecutive_failures_count+1>=10 AND first_failure_at<? THEN 0 ELSE active END,first_failure_at=CASE WHEN consecutive_failures_count=0 THEN ? ELSE first_failure_at END,consecutive_failures_count=consecutive_failures_count+1,updated_at=? WHERE id=?")
            .bind(threshold).bind(&completed_at).bind(&completed_at).bind(&webhook_id)
            .execute(&mut *tx).await.map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

async fn webhook_worker(app: Arc<App>) {
    let host = env::var("RIZZY_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3007".into());
    let semaphore = Arc::new(tokio::sync::Semaphore::new(8));
    let mut interval = tokio::time::interval(Duration::from_millis(250));
    let mut ticks = 0u32;
    loop {
        interval.tick().await;
        if ticks == 0 {
            let stale = (Utc::now() - chrono::Duration::minutes(15))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            if let Err(error) = sqlx::query("UPDATE webhook_deliveries SET state='pending' WHERE state='in_progress' AND updated_at<?")
                .bind(stale).execute(&app.db).await { eprintln!("webhook recovery failed: {error}"); }
        }
        ticks = (ticks + 1) % 240;
        while let Ok(permit) = semaphore.clone().try_acquire_owned() {
            let claimed = sqlx::query_scalar::<_,String>("UPDATE webhook_deliveries SET state='in_progress',updated_at=? WHERE id=(SELECT id FROM webhook_deliveries WHERE state='pending' ORDER BY created_at,id LIMIT 1) AND state='pending' RETURNING id")
                .bind(now()).fetch_optional(&app.db).await;
            match claimed {
                Ok(Some(delivery_id)) => {
                    let app = app.clone();
                    let host = host.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        if let Err(error) =
                            process_webhook_delivery(&app, &delivery_id, &host).await
                        {
                            eprintln!("webhook delivery {delivery_id} failed: {}", error.1);
                            let _ = sqlx::query("UPDATE webhook_deliveries SET state='errored',response=?,updated_at=? WHERE id=?")
                                .bind(json!({"error":"delivery_failed"}).to_string()).bind(now()).bind(&delivery_id)
                                .execute(&app.db).await;
                        }
                    });
                }
                Ok(None) => break,
                Err(error) => {
                    eprintln!("webhook claim failed: {error}");
                    break;
                }
            }
        }
    }
}
async fn board_json(app: &App, p: &Principal, r: &SqliteRow, host: &str) -> Result<Value> {
    let creator: String = r.get("creator_id");
    Ok(board_json_row(
        p,
        r,
        host,
        user_json(app, p, &creator, host).await?,
    ))
}
fn board_json_row(p: &Principal, r: &SqliteRow, host: &str, creator: Value) -> Value {
    let id: String = r.get("id");
    let all = r.get::<i64, _>("all_access") != 0;
    let mut value = json!({"id":id,"name":r.get::<String,_>("name"),"all_access":all,"created_at":r.get::<String,_>("created_at"),"auto_postpone_period_in_days":r.get::<i64,_>("auto_postpone_period_in_days"),"url":format!("{host}/{}/boards/{id}",p.slug),"creator":creator});
    if let Some(key) = r.get::<Option<String>, _>("publication_key") {
        let description: String = r.get("public_description");
        value["public_description"] = json!(rich_text::plain_text(&description));
        value["public_description_html"] = json!(rich_html(&description));
        value["public_url"] = json!(format!("{host}/{}/public/boards/{key}", p.slug));
    }
    value
}
async fn board_detail(app: &App, p: &Principal, r: &SqliteRow, host: &str) -> Result<Value> {
    let mut out = board_json(app, p, r, host).await?;
    if r.get::<i64, _>("all_access") == 0 {
        let ids: Vec<String> =
            sqlx::query_scalar("SELECT user_id FROM accesses WHERE board_id=? ORDER BY user_id")
                .bind(r.get::<String, _>("id"))
                .fetch_all(&app.db)
                .await
                .map_err(db_err)?;
        out["user_ids"] = json!(ids);
    }
    Ok(out)
}
fn column_json(p: &Principal, r: &SqliteRow, host: &str) -> Value {
    let id: String = r.get("id");
    let board: String = r.get("board_id");
    let stored: String = r.get("color");
    let name = match stored.as_str() {
        "var(--color-card-1)" => "Gray",
        "var(--color-card-2)" => "Tan",
        "var(--color-card-3)" => "Yellow",
        "var(--color-card-4)" => "Lime",
        "var(--color-card-5)" => "Aqua",
        "var(--color-card-6)" => "Violet",
        "var(--color-card-7)" => "Purple",
        "var(--color-card-8)" => "Pink",
        _ => "Blue",
    };
    let value = if name == "Blue" {
        "var(--color-card-default)"
    } else {
        stored.as_str()
    };
    json!({"id":id,"name":r.get::<String,_>("name"),"color":{"name":name,"value":value},"created_at":r.get::<String,_>("created_at"),"cards_url":format!("{host}/{}/boards/{board}/columns/{id}/cards",p.slug)})
}
async fn card_json(
    app: &App,
    p: &Principal,
    r: &SqliteRow,
    host: &str,
    with_steps: bool,
) -> Result<Value> {
    let id: String = r.get("id");
    let board_id: String = r.get("board_id");
    let creator: String = r.get("creator_id");
    let description: String = r.get("description");
    let (description_plain, description_html, has_attachments) =
        storage::render_rich_text(app, &p.account_id, p.slug, &description).await?;
    let number: i64 = r.get("number");
    let tags: Vec<String> = sqlx::query_scalar("SELECT t.title FROM tags t JOIN taggings g ON g.tag_id=t.id WHERE g.card_id=? ORDER BY t.title").bind(&id).fetch_all(&app.db).await.map_err(db_err)?;
    let b = board(app, p, &board_id).await?;
    let mut out = json!({"id":id,"number":number,"title":r.get::<String,_>("title"),"status":r.get::<String,_>("status"),"description":description_plain,"description_html":description_html,"image_url":null,"has_attachments":has_attachments,"tags":tags,"closed":r.get::<i64,_>("closed")!=0,"postponed":r.get::<i64,_>("postponed")!=0,"golden":r.get::<i64,_>("golden")!=0,"last_active_at":r.get::<String,_>("last_active_at"),"created_at":r.get::<String,_>("created_at"),"url":format!("{host}/{}/cards/{number}",p.slug),"board":board_json(app,p,&b,host).await?,"creator":user_json(app,p,&creator,host).await?,"assignees":[],"has_more_assignees":false,"comments_url":format!("{host}/{}/cards/{number}/comments",p.slug),"reactions_url":format!("{host}/{}/cards/{number}/reactions",p.slug)});
    out["image_url"] = storage::image_url_for_card(app, p, &id).await?;
    let assigned = sqlx::query(
        "SELECT assignee_id FROM assignments WHERE card_id=? ORDER BY created_at,id LIMIT 6",
    )
    .bind(&id)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let mut assignees = Vec::new();
    for row in assigned.iter().take(5) {
        assignees.push(user_json(app, p, &row.get::<String, _>("assignee_id"), host).await?);
    }
    out["assignees"] = json!(assignees);
    out["has_more_assignees"] = json!(assigned.len() > 5);
    if let Some(column_id) = r.get::<Option<String>, _>("column_id") {
        if let Some(c) = sqlx::query("SELECT * FROM columns WHERE id=? AND board_id=?")
            .bind(column_id)
            .bind(&board_id)
            .fetch_optional(&app.db)
            .await
            .map_err(db_err)?
        {
            out["column"] = column_json(p, &c, host);
        }
    }
    if with_steps {
        let rows = sqlx::query(
            "SELECT id,content,completed FROM steps WHERE card_id=? ORDER BY created_at,id",
        )
        .bind(&id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
        out["steps"]=json!(rows.iter().map(|s|json!({"id":s.get::<String,_>("id"),"content":s.get::<String,_>("content"),"completed":s.get::<i64,_>("completed")!=0})).collect::<Vec<_>>());
    }
    Ok(out)
}

async fn cards_json_batch(
    app: &App,
    p: &Principal,
    cards: &[SqliteRow],
    host: &str,
) -> Result<Vec<Value>> {
    if cards.is_empty() {
        return Ok(Vec::new());
    }
    let card_ids = unique_ids(cards.iter().map(|c| c.get::<String, _>("id")));
    let board_ids = unique_ids(cards.iter().map(|c| c.get::<String, _>("board_id")));
    let column_ids = unique_ids(
        cards
            .iter()
            .filter_map(|c| c.get::<Option<String>, _>("column_id")),
    );
    let (boards, columns, assignments, tags) = tokio::try_join!(
        rows_for_ids(app, "SELECT * FROM boards WHERE id IN (", &board_ids),
        rows_for_ids(app, "SELECT * FROM columns WHERE id IN (", &column_ids),
        assignment_rows_for_cards(app, &card_ids),
        rows_for_ids(
            app,
            "SELECT g.card_id,t.title FROM taggings g JOIN tags t ON t.id=g.tag_id WHERE g.card_id IN (",
            &card_ids
        )
    )?;
    let user_ids = unique_ids(
        cards
            .iter()
            .map(|c| c.get::<String, _>("creator_id"))
            .chain(boards.iter().map(|b| b.get::<String, _>("creator_id")))
            .chain(
                assignments
                    .iter()
                    .map(|a| a.get::<String, _>("assignee_id")),
            ),
    );
    let users=rows_for_ids(app,"SELECT u.*,i.email_address FROM users u JOIN identities i ON i.id=u.identity_id WHERE u.id IN (",&user_ids).await?;
    let users: HashMap<String, Value> = users
        .iter()
        .map(|u| (u.get::<String, _>("id"), user_json_row(p, u, host)))
        .collect();
    let mut board_values = HashMap::new();
    for board in &boards {
        let creator = users
            .get(&board.get::<String, _>("creator_id"))
            .cloned()
            .ok_or_else(missing)?;
        board_values.insert(
            board.get::<String, _>("id"),
            board_json_row(p, board, host, creator),
        );
    }
    let column_values: HashMap<String, Value> = columns
        .iter()
        .map(|c| (c.get::<String, _>("id"), column_json(p, c, host)))
        .collect();
    let mut tag_values: HashMap<String, Vec<String>> = HashMap::new();
    for tag in &tags {
        tag_values
            .entry(tag.get("card_id"))
            .or_default()
            .push(tag.get("title"));
    }
    for tags in tag_values.values_mut() {
        tags.sort();
    }
    let image_values = storage::image_urls_for_cards(app, p, &card_ids).await?;
    let mut assignee_values: HashMap<String, Vec<String>> = HashMap::new();
    for assignment in &assignments {
        assignee_values
            .entry(assignment.get("card_id"))
            .or_default()
            .push(assignment.get("assignee_id"));
    }
    let mut out = Vec::with_capacity(cards.len());
    for card in cards {
        let card_id: String = card.get("id");
        let board_id: String = card.get("board_id");
        let number: i64 = card.get("number");
        let description: String = card.get("description");
        let (description_plain, description_html, has_attachments) =
            storage::render_rich_text(app, &p.account_id, p.slug, &description).await?;
        let assigned = assignee_values.get(&card_id).cloned().unwrap_or_default();
        let assignees: Vec<Value> = assigned
            .iter()
            .take(5)
            .filter_map(|id| users.get(id).cloned())
            .collect();
        let mut value = json!({"id":card_id,"number":number,"title":card.get::<String,_>("title"),"status":card.get::<String,_>("status"),
            "description":description_plain,"description_html":description_html,"image_url":image_values.get(&card_id).cloned().unwrap_or(Value::Null),"has_attachments":has_attachments,
            "tags":tag_values.get(&card_id).cloned().unwrap_or_default(),"closed":card.get::<i64,_>("closed")!=0,
            "postponed":card.get::<i64,_>("postponed")!=0,"golden":card.get::<i64,_>("golden")!=0,
            "last_active_at":card.get::<String,_>("last_active_at"),"created_at":card.get::<String,_>("created_at"),
            "url":format!("{host}/{}/cards/{number}",p.slug),"board":board_values.get(&board_id).cloned().ok_or_else(missing)?,
            "creator":users.get(&card.get::<String,_>("creator_id")).cloned().ok_or_else(missing)?,
            "assignees":assignees,"has_more_assignees":assigned.len()>5,
            "comments_url":format!("{host}/{}/cards/{number}/comments",p.slug),
            "reactions_url":format!("{host}/{}/cards/{number}/reactions",p.slug)});
        if let Some(column_id) = card.get::<Option<String>, _>("column_id") {
            if let Some(column) = column_values.get(&column_id) {
                value["column"] = column.clone();
            }
        }
        out.push(value);
    }
    Ok(out)
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let url =
        env::var("RIZZY_DATABASE_URL").unwrap_or_else(|_| "sqlite://data/rizzy.sqlite3".into());
    if url.starts_with("sqlite://data/") {
        std::fs::create_dir_all("data")?;
    }
    let options = url
        .parse::<SqliteConnectOptions>()?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let pool_size = env::var("RIZZY_DB_POOL_SIZE")
        .ok()
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(12)
        .clamp(1, 64);
    let db = SqlitePoolOptions::new()
        .max_connections(pool_size)
        .connect_with(options)
        .await?;
    sqlx::raw_sql(include_str!("../schema.sql"))
        .execute(&db)
        .await?;
    sqlx::query("INSERT OR IGNORE INTO user_attachables(user_id,account_id,sgid) SELECT id,account_id,lower(hex(randomblob(32))) FROM users")
        .execute(&db).await?;
    migrate_notification_sources(&db).await?;
    backfill_system_users(&db).await?;
    let blob_columns = sqlx::query("PRAGMA table_info(storage_blobs)")
        .fetch_all(&db)
        .await?;
    for (column, definition) in [
        ("width", "INTEGER"),
        ("height", "INTEGER"),
        ("metadata", "TEXT NOT NULL DEFAULT '{}'"),
    ] {
        if !blob_columns
            .iter()
            .any(|row| row.get::<String, _>("name") == column)
        {
            sqlx::query(&format!(
                "ALTER TABLE storage_blobs ADD COLUMN {column} {definition}"
            ))
            .execute(&db)
            .await?;
        }
    }
    let card_columns = sqlx::query("PRAGMA table_info(cards)")
        .fetch_all(&db)
        .await?;
    if !card_columns
        .iter()
        .any(|row| row.get::<String, _>("name") == "due_on")
    {
        sqlx::query("ALTER TABLE cards ADD COLUMN due_on TEXT")
            .execute(&db)
            .await?;
    }
    let download_columns = sqlx::query("PRAGMA table_info(storage_download_tokens)")
        .fetch_all(&db)
        .await?;
    for (column, default) in [("disposition", "inline"), ("variant", "original")] {
        if !download_columns
            .iter()
            .any(|row| row.get::<String, _>("name") == column)
        {
            sqlx::query(&format!(
                "ALTER TABLE storage_download_tokens ADD COLUMN {column} TEXT NOT NULL DEFAULT '{default}'"
            ))
            .execute(&db)
            .await?;
        }
    }
    sqlx::query("INSERT OR IGNORE INTO closures(card_id,user_id,created_at) SELECT c.id,COALESCE((SELECT e.creator_id FROM events e WHERE e.card_id=c.id AND e.action='card_closed' ORDER BY e.created_at DESC,e.id DESC LIMIT 1),c.creator_id),COALESCE((SELECT e.created_at FROM events e WHERE e.card_id=c.id AND e.action='card_closed' ORDER BY e.created_at DESC,e.id DESC LIMIT 1),c.updated_at) FROM cards c WHERE c.closed=1 AND NOT EXISTS(SELECT 1 FROM closures z WHERE z.card_id=c.id)")
        .execute(&db).await?;
    let user_columns = sqlx::query("PRAGMA table_info(users)")
        .fetch_all(&db)
        .await?;
    if !user_columns
        .iter()
        .any(|r| r.get::<String, _>("name") == "verified_at")
    {
        sqlx::query("ALTER TABLE users ADD COLUMN verified_at TEXT")
            .execute(&db)
            .await?;
        sqlx::query("UPDATE users SET verified_at=created_at WHERE role='owner'")
            .execute(&db)
            .await?;
    }
    migrate_deactivated_users(&db).await?;
    for table in ["cards", "comments"] {
        let columns = sqlx::query(&format!("PRAGMA table_info({table})"))
            .fetch_all(&db)
            .await?;
        if !columns
            .iter()
            .any(|row| row.get::<String, _>("name") == "search_text")
        {
            sqlx::query(&format!(
                "ALTER TABLE {table} ADD COLUMN search_text TEXT NOT NULL DEFAULT ''"
            ))
            .execute(&db)
            .await?;
        }
    }
    let access_columns = sqlx::query("PRAGMA table_info(accesses)")
        .fetch_all(&db)
        .await?;
    if !access_columns
        .iter()
        .any(|r| r.get::<String, _>("name") == "accessed_at")
    {
        sqlx::query("ALTER TABLE accesses ADD COLUMN accessed_at TEXT")
            .execute(&db)
            .await?;
    }
    let webhook_columns = sqlx::query("PRAGMA table_info(webhooks)")
        .fetch_all(&db)
        .await?;
    if !webhook_columns
        .iter()
        .any(|r| r.get::<String, _>("name") == "consecutive_failures_count")
    {
        sqlx::query(
            "ALTER TABLE webhooks ADD COLUMN consecutive_failures_count INTEGER NOT NULL DEFAULT 0",
        )
        .execute(&db)
        .await?;
    }
    if !webhook_columns
        .iter()
        .any(|r| r.get::<String, _>("name") == "first_failure_at")
    {
        sqlx::query("ALTER TABLE webhooks ADD COLUMN first_failure_at TEXT")
            .execute(&db)
            .await?;
    }
    let comment_columns = sqlx::query("PRAGMA table_info(comments)")
        .fetch_all(&db)
        .await?;
    if !comment_columns
        .iter()
        .any(|r| r.get::<String, _>("name") == "updated_at")
    {
        sqlx::query("ALTER TABLE comments ADD COLUMN updated_at TEXT")
            .execute(&db)
            .await?;
        sqlx::query("UPDATE comments SET updated_at=created_at WHERE updated_at IS NULL")
            .execute(&db)
            .await?;
    }
    let tag_columns = sqlx::query("PRAGMA table_info(tags)")
        .fetch_all(&db)
        .await?;
    if !tag_columns
        .iter()
        .any(|r| r.get::<String, _>("name") == "created_at")
    {
        sqlx::query("ALTER TABLE tags ADD COLUMN created_at TEXT")
            .execute(&db)
            .await?;
        sqlx::query("UPDATE tags SET created_at=? WHERE created_at IS NULL")
            .bind(now())
            .execute(&db)
            .await?;
    }
    let board_columns = sqlx::query("PRAGMA table_info(boards)")
        .fetch_all(&db)
        .await?;
    if !board_columns
        .iter()
        .any(|r| r.get::<String, _>("name") == "public_description")
    {
        sqlx::query("ALTER TABLE boards ADD COLUMN public_description TEXT NOT NULL DEFAULT ''")
            .execute(&db)
            .await?;
    }
    if !board_columns
        .iter()
        .any(|r| r.get::<String, _>("name") == "publication_key")
    {
        sqlx::query("ALTER TABLE boards ADD COLUMN publication_key TEXT")
            .execute(&db)
            .await?;
    }
    if !board_columns
        .iter()
        .any(|r| r.get::<String, _>("name") == "entropy_override")
    {
        sqlx::query("ALTER TABLE boards ADD COLUMN entropy_override INTEGER NOT NULL DEFAULT 0")
            .execute(&db)
            .await?;
        sqlx::query("UPDATE boards SET entropy_override=1 WHERE auto_postpone_period_in_days!=30")
            .execute(&db)
            .await?;
    }
    let access_columns = sqlx::query("PRAGMA table_info(accesses)")
        .fetch_all(&db)
        .await?;
    if !access_columns
        .iter()
        .any(|r| r.get::<String, _>("name") == "involvement")
    {
        sqlx::query(
            "ALTER TABLE accesses ADD COLUMN involvement TEXT NOT NULL DEFAULT 'access_only'",
        )
        .execute(&db)
        .await?;
        sqlx::query("UPDATE accesses SET involvement='watching' WHERE EXISTS(SELECT 1 FROM boards b WHERE b.id=accesses.board_id AND b.creator_id=accesses.user_id)")
            .execute(&db).await?;
    }
    sqlx::query("INSERT OR IGNORE INTO accesses(board_id,user_id,involvement) SELECT b.id,u.id,CASE WHEN b.creator_id=u.id THEN 'watching' ELSE 'access_only' END FROM boards b JOIN users u ON u.account_id=b.account_id WHERE b.all_access=1 AND u.active=1 AND u.role!='system'")
        .execute(&db).await?;
    let account_columns = sqlx::query("PRAGMA table_info(accounts)")
        .fetch_all(&db)
        .await?;
    if !account_columns
        .iter()
        .any(|r| r.get::<String, _>("name") == "auto_postpone_period_in_days")
    {
        sqlx::query("ALTER TABLE accounts ADD COLUMN auto_postpone_period_in_days INTEGER NOT NULL DEFAULT 30")
            .execute(&db).await?;
    }
    sqlx::query(
        "CREATE UNIQUE INDEX IF NOT EXISTS boards_publication_key ON boards(publication_key)",
    )
    .execute(&db)
    .await?;
    let missing_join_codes=sqlx::query("SELECT a.id FROM accounts a WHERE NOT EXISTS(SELECT 1 FROM account_join_codes j WHERE j.account_id=a.id)")
        .fetch_all(&db).await?;
    for account in missing_join_codes {
        let mut tx = db.begin().await?;
        insert_join_code_tx(&mut tx, &account.get::<String, _>("id")).await?;
        tx.commit().await?;
    }
    if env::args().nth(1).as_deref() == Some("seed") {
        seed(&db).await?;
        return Ok(());
    }
    let app = Arc::new(App {
        db,
        write_lock: tokio::sync::Mutex::new(()),
        auth_lock: tokio::sync::Mutex::new(()),
        export_notify: tokio::sync::Notify::new(),
        import_notify: tokio::sync::Notify::new(),
    });
    let plain_search_backfilled: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM search_migrations WHERE key='fts_plain_v2'")
            .fetch_one(&app.db)
            .await?;
    if plain_search_backfilled == 0 {
        let cards = sqlx::query("SELECT c.id,c.account_id,c.description,a.external_account_id AS slug FROM cards c JOIN accounts a ON a.id=c.account_id")
            .fetch_all(&app.db).await?;
        let mut card_text = Vec::with_capacity(cards.len());
        for row in cards {
            let (plain, _, _) = storage::render_rich_text(
                &app,
                &row.get::<String, _>("account_id"),
                row.get("slug"),
                &row.get::<String, _>("description"),
            )
            .await
            .map_err(|error| std::io::Error::other(error.1))?;
            card_text.push((row.get::<String, _>("id"), search_content(plain)));
        }
        let comments = sqlx::query("SELECT m.id,m.body,c.account_id,a.external_account_id AS slug FROM comments m JOIN cards c ON c.id=m.card_id JOIN accounts a ON a.id=c.account_id")
            .fetch_all(&app.db).await?;
        let mut comment_text = Vec::with_capacity(comments.len());
        for row in comments {
            let (plain, _, _) = storage::render_rich_text(
                &app,
                &row.get::<String, _>("account_id"),
                row.get("slug"),
                &row.get::<String, _>("body"),
            )
            .await
            .map_err(|error| std::io::Error::other(error.1))?;
            comment_text.push((row.get::<String, _>("id"), search_content(plain)));
        }
        let mut tx = app.db.begin().await?;
        for (id, content) in card_text {
            sqlx::query("UPDATE cards SET search_text=? WHERE id=?")
                .bind(content)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        for (id, content) in comment_text {
            sqlx::query("UPDATE comments SET search_text=? WHERE id=?")
                .bind(content)
                .bind(id)
                .execute(&mut *tx)
                .await?;
        }
        sqlx::raw_sql(include_str!("../search_triggers.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM card_search")
            .execute(&mut *tx)
            .await?;
        sqlx::query("DELETE FROM comment_search")
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO card_search(rowid,title,description) SELECT rowid,title,search_text FROM cards WHERE status='published'")
            .execute(&mut *tx).await?;
        sqlx::query("INSERT INTO comment_search(rowid,body) SELECT m.rowid,m.search_text FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.status='published'")
            .execute(&mut *tx).await?;
        sqlx::query("INSERT OR IGNORE INTO search_migrations(key) VALUES('fts_plain_v2')")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    if env::args().nth(1).as_deref() == Some("deliver-notifications") {
        let count = notification_mail::deliver_due(&app)
            .await
            .map_err(|error| std::io::Error::other(error.1))?;
        println!("Delivered {count} notification bundles");
        return Ok(());
    }
    if env::args().nth(1).as_deref() == Some("deliver-push") {
        let count = push_delivery::deliver_due(&app)
            .await
            .map_err(|error| std::io::Error::other(error.1))?;
        println!("Delivered {count} push jobs");
        return Ok(());
    }
    if env::var("RIZZY_DISABLE_WEBHOOK_DELIVERY").as_deref() != Ok("1") {
        tokio::spawn(webhook_worker(app.clone()));
    }
    tokio::spawn(exports::worker(app.clone()));
    tokio::spawn(imports::worker(app.clone()));
    tokio::spawn(notification_mail::worker(app.clone()));
    if env::var("RIZZY_DISABLE_PUSH_DELIVERY").as_deref() != Ok("1") {
        tokio::spawn(push_delivery::worker(app.clone()));
    }
    let maintenance_db = app.db.clone();
    let maintenance_app = app.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            let expiration = now();
            let attempts_cutoff = (Utc::now() - chrono::Duration::minutes(15))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            let transfer_cutoff = (Utc::now() - chrono::Duration::hours(4))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            for (statement, cutoff) in [
                ("DELETE FROM magic_links WHERE expires_at<?", &expiration),
                (
                    "DELETE FROM email_change_requests WHERE expires_at<?",
                    &expiration,
                ),
                (
                    "DELETE FROM pending_authentications WHERE expires_at<?",
                    &expiration,
                ),
                (
                    "DELETE FROM passkey_challenges WHERE expires_at<?",
                    &expiration,
                ),
                (
                    "DELETE FROM notification_unsubscribe_tokens WHERE expires_at<?",
                    &expiration,
                ),
                (
                    "DELETE FROM storage_download_tokens WHERE expires_at<?",
                    &expiration,
                ),
                (
                    "DELETE FROM authentication_attempts WHERE created_at<?",
                    &attempts_cutoff,
                ),
                (
                    "DELETE FROM access_token_reveals WHERE expires_at<?",
                    &expiration,
                ),
                (
                    "DELETE FROM identity_transfers WHERE created_at<?",
                    &transfer_cutoff,
                ),
            ] {
                if let Err(error) = sqlx::query(statement)
                    .bind(cutoff)
                    .execute(&maintenance_db)
                    .await
                {
                    eprintln!("authentication cleanup failed: {error}");
                }
            }
            let delivered_cutoff = (Utc::now() - chrono::Duration::days(30))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            if let Err(error) =
                sqlx::query("DELETE FROM push_deliveries WHERE status=2 AND updated_at<?")
                    .bind(delivered_cutoff)
                    .execute(&maintenance_db)
                    .await
            {
                eprintln!("push delivery cleanup failed: {error}");
            }
            if let Err(error) = account_cancellation::incinerate_due(&maintenance_app).await {
                eprintln!("account incineration failed: {error}");
            }
        }
    });
    let routes = Router::new()
        .route("/up", get(|| async { "ok" }))
        .route("/", get(home))
        .route("/{account}/", get(events_page::index))
        .route("/{account}/landing", get(landing))
        .route(
            "/client_configurations/android_v1.json",
            get(android_config),
        )
        .route("/client_configurations/ios_v1.json", get(ios_config))
        .route("/manifest.json", get(pwa::manifest))
        .route("/service-worker", get(pwa::service_worker))
        .route("/service-worker.js", get(pwa::service_worker))
        .route("/assets/pwa/{filename}", get(pwa::asset))
        .route("/app-icon.png", get(pwa::icon))
        .route("/app-icon-192.png", get(pwa::icon))
        .route("/app-icon-192-maskable.png", get(pwa::icon))
        .route("/session", post(request_magic_link).delete(logout_session))
        .route("/session/new", get(new_session))
        .route("/session/passkey", post(passkeys::sign_in))
        .route("/session/menu", get(account_cancellation::menu))
        .route("/my/passkey_challenge", post(passkeys::challenge))
        .route("/my/passkeys", get(passkeys::index).post(passkeys::register))
        .route("/my/passkeys/{id}/edit", get(passkeys::edit))
        .route(
            "/my/passkeys/{id}",
            axum::routing::put(passkeys::update)
                .patch(passkeys::update)
                .post(passkeys::form_update_or_delete)
                .delete(passkeys::destroy),
        )
        .route(
            "/session/transfers/{token}",
            get(session_transfer::show)
                .put(session_transfer::redeem)
                .patch(session_transfer::redeem)
                .post(session_transfer::redeem),
        )
        .route("/session/magic_link", get(show_magic_link).post(complete_magic_link))
        .route("/signup", get(|| async { (StatusCode::FOUND,[(header::LOCATION,"/signup/new")]) }).post(request_signup))
        .route("/signup/new", get(new_signup))
        .route("/signup/completion", post(complete_signup))
        .route("/signup/completion/new", get(new_signup_completion))
        .route("/my/identity", get(my_identity))
        .route("/account/imports/new", get(imports::new))
        .route("/account/imports", post(imports::create).layer(axum::extract::DefaultBodyLimit::max(512 * 1024 * 1024)))
        .route("/my/access_tokens", get(list_tokens).post(create_token))
        .route("/my/access_tokens/new", get(new_token))
        .route("/{account}/users/{user}/push_subscriptions", get(push_subscriptions::index).post(push_subscriptions::create))
        .route("/{account}/users/{user}/push_subscriptions/{subscription}", axum::routing::delete(push_subscriptions::destroy).post(push_subscriptions::form_destroy))
        .route(
            "/my/access_tokens/{id}",
            get(show_token).post(delete_token).delete(delete_token),
        )
        .route("/{account}/my/identity", get(my_identity))
        .route("/{account}/my/menu", get(my_menu::show))
        .route("/{account}/transfer_token", post(session_transfer::create))
        .route("/{account}/qr_codes/{token}", get(session_transfer::qr_code))
        .route("/assets/system_user.png", get(storage::system_avatar))
        .route("/assets/lexxy/{filename}", get(lexxy_asset))
        .route(
            "/{account}/rails/active_storage/direct_uploads",
            post(storage::direct_upload),
        )
        .route(
            "/{account}/rails/active_storage/disk/{token}",
            axum::routing::put(storage::upload_file),
        )
        .route(
            "/{account}/rails/active_storage/disk/{token}/{filename}",
            get(storage::download_file),
        )
        .route(
            "/{account}/rails/active_storage/blobs/redirect/{signed_id}/{filename}",
            get(storage::show_blob),
        )
        .route(
            "/{account}/rails/active_storage/representations/redirect/{signed_id}/thumb/{filename}",
            get(storage::show_avatar_representation),
        )
        .route(
            "/{account}/rails/active_storage/representations/redirect/{signed_id}/preview/{filename}",
            get(storage::show_blob),
        )
        .route(
            "/{account}/rails/active_storage/disk/thumb/{token}/{filename}",
            get(storage::download_avatar_thumbnail),
        )
        .route(
            "/{account}/my/timezone",
            axum::routing::put(update_timezone).patch(update_timezone),
        )
        .route(
            "/{account}/client_configurations/android_v1.json",
            get(android_config),
        )
        .route(
            "/{account}/client_configurations/ios_v1.json",
            get(ios_config),
        )
        .route(
            "/{account}/my/access_tokens",
            get(list_tokens).post(create_token),
        )
        .route("/{account}/my/access_tokens/new", get(new_token))
        .route(
            "/{account}/my/access_tokens/{id}",
            get(show_token).post(delete_token).delete(delete_token),
        )
        .route("/{account}/users", get(list_users))
        .route("/{account}/prompts/users", get(prompt_users))
        .route("/{account}/prompts/cards", get(prompt_cards))
        .route(
            "/{account}/prompts/boards/{board}/users",
            get(prompt_board_users),
        )
        .route(
            "/{account}/users/{user}/data_exports",
            post(exports::create_user),
        )
        .route(
            "/{account}/users/{user}/data_exports/{id}",
            get(exports::show_user),
        )
        .route("/{account}/account/exports", post(exports::create_account))
        .route("/{account}/account/imports/{id}", get(imports::show))
        .route(
            "/{account}/account/exports/{id}",
            get(exports::show_account),
        )
        .route("/{account}/exports/{id}/download", get(exports::download))
        .route(
            "/{account}/account/settings",
            get(show_account).post(update_account).put(update_account).patch(update_account),
        )
        .route("/{account}/account/cancellation", post(account_cancellation::create))
        .route(
            "/{account}/account/entropy",
            post(update_account_entropy).put(update_account_entropy).patch(update_account_entropy),
        )
        .route(
            "/{account}/account/join_code",
            get(show_join_code)
                .post(update_join_code)
                .put(update_join_code)
                .patch(update_join_code)
                .delete(reset_join_code),
        )
        .route("/{account}/account/join_code/edit", get(edit_join_code))
        .route(
            "/{account}/join/{code}",
            get(join_page).post(redeem_join_code),
        )
        .route("/{account}/users/joins", post(complete_join))
        .route("/{account}/users/joins/new", get(new_user_join))
        .route(
            "/{account}/users/verifications/new",
            get(new_user_verification),
        )
        .route("/{account}/users/verifications", post(verify_user))
        .route(
            "/{account}/users/{id}",
            get(show_user)
                .post(update_user)
                .put(update_user)
                .patch(update_user)
                .delete(delete_user),
        )
        .route("/{account}/users/{id}/edit", get(edit_user))
        .route(
            "/{account}/users/{id}/avatar",
            get(storage::show_avatar).delete(storage::delete_avatar),
        )
        .route("/{account}/users/{id}/events", get(events_page::user_events))
        .route(
            "/{account}/users/{id}/role",
            post(update_user_role).put(update_user_role).patch(update_user_role),
        )
        .route(
            "/{account}/users/{id}/email_addresses",
            post(request_email_change),
        )
        .route(
            "/{account}/users/{id}/email_addresses/new",
            get(new_email_change),
        )
        .route(
            "/{account}/users/{id}/email_addresses/{token}/confirmation",
            get(show_email_change_confirmation).post(confirm_email_change),
        )
        .route("/{account}/boards", get(list_boards).post(create_board))
        .route("/{account}/boards/new", get(board_settings::new))
        .route("/{account}/boards/{id}/edit", get(board_settings::edit))
        .route("/{account}/boards/{id}/accesses", get(list_board_accesses))
        .route(
            "/{account}/boards/{id}/involvement",
            axum::routing::put(update_board_involvement).patch(update_board_involvement),
        )
        .route(
            "/{account}/boards/{board}/webhooks",
            get(list_webhooks).post(create_webhook),
        )
        .route(
            "/{account}/boards/{board}/webhooks/new",
            get(webhook_page::new),
        )
        .route(
            "/{account}/boards/{board}/webhooks/{webhook}/edit",
            get(webhook_page::edit),
        )
        .route(
            "/{account}/boards/{board}/webhooks/{webhook}",
            get(show_webhook)
                .post(update_webhook)
                .put(update_webhook)
                .patch(update_webhook)
                .delete(delete_webhook),
        )
        .route(
            "/{account}/boards/{board}/webhooks/{webhook}/activation",
            post(activate_webhook),
        )
        .route(
            "/{account}/boards/{board}/webhooks/{webhook}/deliveries",
            get(list_webhook_deliveries),
        )
        .route(
            "/{account}/boards/{id}/publication",
            post(publish_board).delete(unpublish_board),
        )
        .route(
            "/{account}/boards/{id}/entropy",
            axum::routing::put(update_board_entropy).patch(update_board_entropy),
        )
        .route("/{account}/public/boards/{key}", get(public_board))
        .route(
            "/{account}/public/boards/{key}/columns/stream",
            get(board_views::public_stream),
        )
        .route(
            "/{account}/public/boards/{key}/columns/closed",
            get(board_views::public_closed),
        )
        .route(
            "/{account}/public/boards/{key}/columns/not_now",
            get(board_views::public_not_now),
        )
        .route(
            "/{account}/public/boards/{key}/columns/{column}",
            get(board_views::public_column),
        )
        .route(
            "/{account}/public/boards/{key}/cards/{number}",
            get(public_card),
        )
        .route(
            "/{account}/boards/{id}",
            get(show_board)
                .post(update_board)
                .put(update_board)
                .patch(update_board)
                .delete(delete_board),
        )
        .route(
            "/{account}/boards/{board}/columns",
            get(list_columns).post(create_column),
        )
        .route(
            "/{account}/boards/{board}/columns/stream",
            get(board_views::private_stream),
        )
        .route(
            "/{account}/boards/{board}/columns/closed",
            get(board_views::private_closed),
        )
        .route(
            "/{account}/boards/{board}/columns/not_now",
            get(board_views::private_not_now),
        )
        .route(
            "/{account}/boards/{board}/columns/{column}",
            get(show_column)
                .put(update_column)
                .patch(update_column)
                .delete(delete_column),
        )
        .route(
            "/{account}/boards/{board}/columns/{column}/cards",
            get(column_cards),
        )
        .route(
            "/{account}/columns/{column}/left_position",
            post(move_column_left),
        )
        .route(
            "/{account}/columns/{column}/right_position",
            post(move_column_right),
        )
        .route("/{account}/boards/{board}/cards", post(create_card))
        .route("/{account}/cards", get(list_cards).post(create_account_card))
        .route(
            "/{account}/columns/cards/{card_id}/drops/closure",
            post(drop_card_closure),
        )
        .route(
            "/{account}/columns/cards/{card_id}/drops/not_now",
            post(drop_card_not_now),
        )
        .route(
            "/{account}/columns/cards/{card_id}/drops/stream",
            post(drop_card_stream),
        )
        .route(
            "/{account}/columns/cards/{card_id}/drops/column",
            post(drop_card_column),
        )
        .route("/{account}/filters", post(create_filter))
        .route("/{account}/filters/settings_refresh", post(refresh_filter_settings))
        .route(
            "/{account}/filters/{id}",
            axum::routing::delete(delete_filter).post(delete_filter),
        )
        .route("/{account}/tags", get(list_tags))
        .route("/{account}/search", get(search_cards))
        .route("/{account}/notifications", get(list_notifications))
        .route(
            "/{account}/notifications/settings",
            get(show_notification_settings)
                .put(update_notification_settings)
                .patch(update_notification_settings)
                .post(update_notification_settings),
        )
        .route("/{account}/notifications/unsubscribe/new", get(unsubscribes::new))
        .route("/{account}/notifications/unsubscribe", get(unsubscribes::show).post(unsubscribes::create))
        .route("/{account}/notifications/tray", get(notification_tray))
        .route(
            "/{account}/notifications/bulk_reading",
            post(read_all_notifications),
        )
        .route(
            "/{account}/notifications/{id}/reading",
            post(read_notification).delete(unread_notification),
        )
        .route("/{account}/activities", get(list_activities))
        .route("/{account}/events", get(events_page::index))
        .route("/{account}/events/days", get(events_page::days))
        .route(
            "/{account}/events/day_timeline/columns/{id}",
            get(events_page::column),
        )
        .route(
            "/{account}/cards/{number}",
            get(show_card)
                .put(update_card)
                .patch(update_card)
                .delete(delete_card),
        )
        .route("/{account}/cards/{number}/draft", get(show_draft))
        .route("/{account}/cards/{number}/edit", get(edit_card))
        .route("/{account}/cards/{number}/column/edit", get(card_column_page::edit))
        .route("/{account}/cards/previews", get(card_previews))
        .route("/{account}/cards/previews.turbo_stream", get(card_previews))
        .route("/{account}/cards/{number}/publish", post(publish_card))
        .route(
            "/{account}/cards/{number}/image",
            axum::routing::delete(storage::delete_image),
        )
        .route(
            "/{account}/cards/{number}/closure",
            post(close_card).delete(open_card),
        )
        .route(
            "/{account}/cards/{number}/reading",
            post(read_card).delete(unread_card),
        )
        .route(
            "/{account}/cards/{number}/triage",
            post(triage_card).delete(untriage_card),
        )
        .route(
            "/{account}/cards/{number}/not_now",
            post(postpone_card).delete(unpostpone_card),
        )
        .route(
            "/{account}/cards/{number}/goldness",
            post(gild_card).delete(ungild_card),
        )
        .route(
            "/{account}/cards/{number}/board",
            axum::routing::put(move_card).patch(move_card).post(move_card),
        )
        .route("/{account}/cards/{number}/board/edit", get(card_board_page::edit))
        .route("/{account}/cards/{number}/taggings", post(toggle_tag))
        .route(
            "/{account}/cards/{number}/taggings/new",
            get(tagging_page::new),
        )
        .route(
            "/{account}/cards/{number}/assignments",
            post(toggle_assignment),
        )
        .route(
            "/{account}/cards/{number}/assignments/new",
            get(assignment_page::new),
        )
        .route(
            "/{account}/cards/{number}/self_assignment",
            post(toggle_self_assignment),
        )
        .route(
            "/{account}/cards/{number}/watch",
            get(card_controls::show_watch)
                .post(card_controls::watch)
                .delete(card_controls::unwatch),
        )
        .route(
            "/{account}/cards/{number}/pin",
            get(card_controls::show_pin)
                .post(card_controls::pin)
                .delete(card_controls::unpin),
        )
        .route("/{account}/my/pins", get(list_pins))
        .route(
            "/{account}/cards/{number}/reactions",
            get(list_card_reactions).post(create_card_reaction),
        )
        .route(
            "/{account}/cards/{number}/reactions/new",
            get(reaction_page::card_new),
        )
        .route(
            "/{account}/cards/{number}/reactions/{reaction}",
            axum::routing::delete(delete_card_reaction),
        )
        .route(
            "/{account}/cards/{number}/steps",
            get(list_steps).post(create_step),
        )
        .route(
            "/{account}/cards/{number}/steps/{step}",
            get(show_step)
                .put(update_step)
                .patch(update_step)
                .delete(delete_step),
        )
        .route(
            "/{account}/cards/{number}/steps/{step}/edit",
            get(edit_step),
        )
        .route(
            "/{account}/cards/{number}/comments",
            get(list_comments).post(create_comment),
        )
        .route(
            "/{account}/cards/{number}/comments/{comment}",
            get(show_comment)
                .put(update_comment)
                .patch(update_comment)
                .post(update_or_delete_comment_form)
                .delete(delete_comment),
        )
        .route(
            "/{account}/cards/{number}/comments/{comment}/edit",
            get(comment_edit_page::edit),
        )
        .route(
            "/{account}/cards/{number}/comments/{comment}/reactions",
            get(list_comment_reactions).post(create_comment_reaction),
        )
        .route(
            "/{account}/cards/{number}/comments/{comment}/reactions/new",
            get(reaction_page::comment_new),
        )
        .route(
            "/{account}/cards/{number}/comments/{comment}/reactions/{reaction}",
            axum::routing::delete(delete_comment_reaction),
        )
        .layer(middleware::from_fn_with_state(
            app.clone(),
            register_pwa_on_pages,
        ))
        .with_state(app)
        .layer(middleware::from_fn(mark_request_format));
    let bind = env::var("RIZZY_BIND").unwrap_or_else(|_| "127.0.0.1:3007".into());
    if dev_magic_links() && !bind.starts_with("127.0.0.1:") && !bind.starts_with("[::1]:") {
        return Err("RIZZY_DEV_MAGIC_LINKS requires a loopback bind address".into());
    }
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("Rizzy listening on http://{bind}");
    let routes = MapRequestLayer::new(rewrite_json_uri).layer(routes);
    axum::serve(
        listener,
        routes.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn seed(db: &SqlitePool) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let account_id = id();
    let identity_id = id();
    let user_id = id();
    let system_identity = id();
    let system_id = id();
    let token_id = id();
    let timestamp = now();
    let slug: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(external_account_id),0)+1 FROM accounts")
            .fetch_one(db)
            .await?;
    let email = env::var("RIZZY_SEED_EMAIL").unwrap_or_else(|_| "owner@example.com".into());
    let name = env::var("RIZZY_SEED_NAME").unwrap_or_else(|_| "Owner".into());
    let account = env::var("RIZZY_SEED_ACCOUNT").unwrap_or_else(|_| "Rizzy".into());
    let token = hex::encode(rand::random::<[u8; 32]>());
    let mut tx = db.begin().await?;
    sqlx::query("INSERT INTO accounts(id,external_account_id,name,created_at) VALUES(?,?,?,?)")
        .bind(&account_id)
        .bind(slug)
        .bind(account)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;
    insert_join_code_tx(&mut tx, &account_id).await?;
    sqlx::query("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)")
        .bind(&identity_id)
        .bind(email)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO users(id,account_id,identity_id,name,role,created_at,verified_at) VALUES(?,?,?,?,'owner',?,?)").bind(&user_id).bind(&account_id).bind(&identity_id).bind(name).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)")
        .bind(&system_identity)
        .bind(format!("system+{account_id}@rizzy.invalid"))
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES(?,?,?,'System','system',?)")
        .bind(&system_id).bind(&account_id).bind(&system_identity).bind(&timestamp)
        .execute(&mut *tx).await?;
    sqlx::query("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES(?,?,?,?,?,?)").bind(&token_id).bind(&identity_id).bind(hash(&token)).bind("Bootstrap token").bind("write").bind(&timestamp).execute(&mut *tx).await?;
    tx.commit().await?;
    println!("Account: {slug}\nBearer token: {token}");
    Ok(())
}

async fn backfill_system_users(db: &SqlitePool) -> std::result::Result<(), sqlx::Error> {
    let accounts = sqlx::query("SELECT a.id FROM accounts a WHERE NOT EXISTS(SELECT 1 FROM users u WHERE u.account_id=a.id AND u.role='system')")
        .fetch_all(db).await?;
    for account in accounts {
        let account_id: String = account.get("id");
        let timestamp = now();
        let email = format!("system+{account_id}@rizzy.invalid");
        let mut tx = db.begin().await?;
        sqlx::query("INSERT OR IGNORE INTO identities(id,email_address,created_at) VALUES(?,?,?)")
            .bind(id())
            .bind(&email)
            .bind(&timestamp)
            .execute(&mut *tx)
            .await?;
        let identity_id: String =
            sqlx::query_scalar("SELECT id FROM identities WHERE email_address=?")
                .bind(&email)
                .fetch_one(&mut *tx)
                .await?;
        sqlx::query("INSERT OR IGNORE INTO users(id,account_id,identity_id,name,role,created_at) VALUES(?,?,?,'System','system',?)")
            .bind(id()).bind(&account_id).bind(identity_id).bind(&timestamp)
            .execute(&mut *tx).await?;
        tx.commit().await?;
    }
    Ok(())
}

async fn migrate_deactivated_users(db: &SqlitePool) -> std::result::Result<(), sqlx::Error> {
    let mut connection = db.acquire().await?;
    let columns = sqlx::query("PRAGMA table_info(users)")
        .fetch_all(&mut *connection)
        .await?;
    if !columns.iter().any(|row| {
        row.get::<String, _>("name") == "identity_id" && row.get::<i64, _>("notnull") == 1
    }) {
        return Ok(());
    }
    sqlx::query("PRAGMA foreign_keys=OFF")
        .execute(&mut *connection)
        .await?;
    let migration: std::result::Result<(), sqlx::Error> = async {
        sqlx::query("BEGIN IMMEDIATE")
            .execute(&mut *connection)
            .await?;
        sqlx::query("CREATE TABLE users_identity_v2 (id TEXT PRIMARY KEY,account_id TEXT NOT NULL REFERENCES accounts(id),identity_id TEXT REFERENCES identities(id),name TEXT NOT NULL,role TEXT NOT NULL DEFAULT 'member',active INTEGER NOT NULL DEFAULT 1,verified_at TEXT,created_at TEXT NOT NULL,UNIQUE(account_id,identity_id))")
            .execute(&mut *connection).await?;
        sqlx::query("INSERT INTO users_identity_v2(id,account_id,identity_id,name,role,active,verified_at,created_at) SELECT id,account_id,identity_id,name,role,active,verified_at,created_at FROM users")
            .execute(&mut *connection).await?;
        sqlx::query("DROP TABLE users")
            .execute(&mut *connection)
            .await?;
        sqlx::query("ALTER TABLE users_identity_v2 RENAME TO users")
            .execute(&mut *connection).await?;
        sqlx::query("CREATE UNIQUE INDEX users_system_account ON users(account_id) WHERE role='system'")
            .execute(&mut *connection).await?;
        sqlx::query("CREATE TRIGGER user_attachables_created AFTER INSERT ON users BEGIN INSERT INTO user_attachables(user_id,account_id,sgid) VALUES(NEW.id,NEW.account_id,lower(hex(randomblob(32)))); END")
            .execute(&mut *connection).await?;
        sqlx::query("COMMIT").execute(&mut *connection).await?;
        Ok(())
    }.await;
    if migration.is_err() {
        let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
    }
    let restored = sqlx::query("PRAGMA foreign_keys=ON")
        .execute(&mut *connection)
        .await;
    migration?;
    restored?;
    let violations = sqlx::query("PRAGMA foreign_key_check")
        .fetch_all(&mut *connection)
        .await?;
    if !violations.is_empty() {
        return Err(sqlx::Error::Protocol(
            "User migration produced broken foreign keys".into(),
        ));
    }
    Ok(())
}

async fn migrate_notification_sources(db: &SqlitePool) -> std::result::Result<(), sqlx::Error> {
    let mut connection = db.acquire().await?;
    let foreign_keys = sqlx::query("PRAGMA foreign_key_list(notifications)")
        .fetch_all(&mut *connection)
        .await?;
    if !foreign_keys
        .iter()
        .any(|row| row.get::<String, _>("from") == "source_id")
    {
        return Ok(());
    }
    sqlx::query("PRAGMA foreign_keys=OFF")
        .execute(&mut *connection)
        .await?;
    let migration: std::result::Result<(),sqlx::Error> = async {
        sqlx::query("BEGIN IMMEDIATE").execute(&mut *connection).await?;
        sqlx::query("CREATE TABLE notifications_source_v2 (id TEXT PRIMARY KEY,account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,creator_id TEXT NOT NULL REFERENCES users(id),card_id TEXT NOT NULL REFERENCES cards(id) ON DELETE CASCADE,source_id TEXT NOT NULL,source_type TEXT NOT NULL DEFAULT 'Event',unread_count INTEGER NOT NULL DEFAULT 1,read_at TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL,UNIQUE(user_id,card_id))")
            .execute(&mut *connection).await?;
        sqlx::query("INSERT INTO notifications_source_v2 SELECT id,account_id,user_id,creator_id,card_id,source_id,source_type,unread_count,read_at,created_at,updated_at FROM notifications")
            .execute(&mut *connection).await?;
        sqlx::query("DROP TRIGGER IF EXISTS notifications_event_delete")
            .execute(&mut *connection).await?;
        sqlx::query("DROP TRIGGER IF EXISTS notifications_mention_delete")
            .execute(&mut *connection).await?;
        sqlx::query("DROP TABLE notifications").execute(&mut *connection).await?;
        sqlx::query("ALTER TABLE notifications_source_v2 RENAME TO notifications")
            .execute(&mut *connection).await?;
        sqlx::query("CREATE INDEX notifications_user_read ON notifications(user_id,read_at,updated_at DESC)")
            .execute(&mut *connection).await?;
        sqlx::query("CREATE TRIGGER notifications_event_delete AFTER DELETE ON events BEGIN DELETE FROM notifications WHERE source_type='Event' AND source_id=OLD.id; END")
            .execute(&mut *connection).await?;
        sqlx::query("CREATE TRIGGER notifications_mention_delete AFTER DELETE ON mentions BEGIN DELETE FROM notifications WHERE source_type='Mention' AND source_id=OLD.id; END")
            .execute(&mut *connection).await?;
        sqlx::query("COMMIT").execute(&mut *connection).await?;
        Ok(())
    }.await;
    if migration.is_err() {
        let _ = sqlx::query("ROLLBACK").execute(&mut *connection).await;
    }
    let restored = sqlx::query("PRAGMA foreign_keys=ON")
        .execute(&mut *connection)
        .await;
    migration?;
    restored?;
    Ok(())
}

fn future_minutes(minutes: i64) -> String {
    (Utc::now() + chrono::Duration::minutes(minutes))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}
fn valid_email(email: &str) -> bool {
    !email.contains(['\r', '\n', ' '])
        && email.len() <= 254
        && email.split_once('@').is_some_and(|(local, domain)| {
            !local.is_empty() && domain.contains('.') && !domain.starts_with('.')
        })
}
fn magic_code() -> String {
    const ALPHABET: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
    let mut rng = rand::rng();
    (0..6)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}
fn normalize_magic_code(code: &str) -> String {
    code.to_ascii_uppercase()
        .replace('O', "0")
        .replace(['I', 'L'], "1")
        .chars()
        .filter(|c| "0123456789ABCDEFGHJKMNPQRSTVWXYZ".contains(*c))
        .collect()
}
async fn auth_attempt(
    app: &App,
    bucket: &str,
    limit: i64,
    window_minutes: i64,
    message: &str,
) -> Result<()> {
    let cutoff = (Utc::now() - chrono::Duration::minutes(window_minutes))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM authentication_attempts WHERE bucket=? AND created_at>=?",
    )
    .bind(bucket)
    .bind(&cutoff)
    .fetch_one(&app.db)
    .await
    .map_err(db_err)?;
    if count >= limit {
        return Err(err(StatusCode::TOO_MANY_REQUESTS, message));
    }
    sqlx::query("INSERT INTO authentication_attempts(id,bucket,created_at) VALUES(?,?,?)")
        .bind(id())
        .bind(bucket)
        .bind(now())
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(())
}
fn dev_magic_links() -> bool {
    env::var("RIZZY_DEV_MAGIC_LINKS").as_deref() == Ok("1")
}
async fn deliver_magic_link(email: &str, code: &str, host: &str) -> Result<()> {
    if dev_magic_links() {
        return Ok(());
    }
    let path = env::var("RIZZY_SENDMAIL").map_err(|_| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "Mail delivery is not configured",
        )
    })?;
    let from = env::var("RIZZY_MAIL_FROM").unwrap_or_else(|_| "no-reply@localhost".into());
    if !valid_email(email) || from.contains(['\r', '\n']) {
        return Err(bad("invalid email address"));
    }
    let message = format!(
        "From: {from}\nTo: {email}\nSubject: Your Rizzy sign-in code\nContent-Type: text/plain; charset=UTF-8\n\nYour Rizzy code is {code}.\n\nIt expires in 15 minutes.\n{host}/session/magic_link?code={code}\n"
    );
    let mut child = Command::new(path)
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
    let status = child
        .wait()
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"))?;
    if !status.success() {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"));
    }
    Ok(())
}
async fn request_magic_link(
    State(app): Shared,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let (body, form) = session_body(&headers, &body)?;
    let mut response = request_magic_link_inner(&app, peer, headers, body, false).await?;
    if form {
        *response.status_mut() = StatusCode::FOUND;
        response.headers_mut().insert(
            header::LOCATION,
            HeaderValue::from_static("/session/magic_link"),
        );
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        *response.body_mut() = Body::empty();
    }
    Ok(response)
}
fn session_body(headers: &HeaderMap, body: &Bytes) -> Result<(Value, bool)> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded");
    if form {
        let fields = url::form_urlencoded::parse(body)
            .map(|(key, value)| (key.into_owned(), Value::String(value.into_owned())))
            .collect::<serde_json::Map<String, Value>>();
        Ok((Value::Object(fields), true))
    } else {
        Ok((
            serde_json::from_slice(body).map_err(|_| bad("Invalid JSON"))?,
            false,
        ))
    }
}
async fn show_magic_link(State(app): Shared, headers: HeaderMap) -> Result<Response> {
    let Some(pending) = cookie_value(&headers, "pending_authentication_token") else {
        return Ok((StatusCode::FOUND, [(header::LOCATION, "/session/new")]).into_response());
    };
    let row = sqlx::query(
        "SELECT email_address FROM pending_authentications WHERE token_hash=? AND expires_at>?",
    )
    .bind(hash(&pending))
    .bind(now())
    .fetch_optional(&app.db)
    .await
    .map_err(db_err)?;
    let Some(row) = row else {
        return Ok((StatusCode::FOUND, [(header::LOCATION, "/session/new")]).into_response());
    };
    let email: String = row.get("email_address");
    let code_hint = if dev_magic_links() {
        let code: Option<String> = sqlx::query_scalar("SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id WHERE i.email_address=? AND m.expires_at>? ORDER BY m.created_at DESC LIMIT 1")
            .bind(&email).bind(now()).fetch_optional(&app.db).await.map_err(db_err)?;
        code.map(|code| {
            format!(
                "<p>Development code: <strong>{}</strong></p>",
                escape(&code)
            )
        })
        .unwrap_or_default()
    } else {
        String::new()
    };
    Ok(Html(format!("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Check your email | Rizzy</title></head><body><main><h1>Check your email</h1><p>Enter the verification code sent to <strong>{}</strong>.</p><form method=\"post\" action=\"/session/magic_link\"><label>Verification code<input name=\"code\" autocomplete=\"one-time-code\" maxlength=\"6\" required autofocus></label><button type=\"submit\">Continue</button></form>{code_hint}</main></body></html>",escape(&email))).into_response())
}
async fn new_signup(State(app): Shared, headers: HeaderMap) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    if identity_auth(&app, &headers).await.is_ok() {
        return Ok((
            StatusCode::FOUND,
            [(header::LOCATION, "/signup/completion/new")],
        )
            .into_response());
    }
    let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    if accounts > 0 && env::var("RIZZY_MULTI_TENANT").as_deref() != Ok("1") {
        return Ok((StatusCode::FOUND, [(header::LOCATION, "/session/new")]).into_response());
    }
    Ok(signup_page::new().into_response())
}
async fn new_signup_completion(State(app): Shared, headers: HeaderMap) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    identity_auth(&app, &headers).await?;
    Ok(signup_page::completion(None).into_response())
}
async fn request_signup(
    State(app): Shared,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    if identity_auth(&app, &headers).await.is_ok() {
        return Ok((
            StatusCode::FOUND,
            [(header::LOCATION, "/signup/completion/new")],
        )
            .into_response());
    }
    let (body, form) = session_body(&headers, &body)?;
    let email = if form {
        string(&body, "signup[email_address]").unwrap_or_default()
    } else {
        required(nested(&body, "signup"), "email_address")?
    };
    if !valid_email(&email) {
        return Ok(StatusCode::UNPROCESSABLE_ENTITY.into_response());
    }
    let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    if accounts > 0 && env::var("RIZZY_MULTI_TENANT").as_deref() != Ok("1") {
        return Ok((StatusCode::FOUND, [(header::LOCATION, "/session/new")]).into_response());
    }
    let mut response =
        request_magic_link_inner(&app, peer, headers, json!({"email_address":email}), true).await?;
    if form {
        *response.status_mut() = StatusCode::FOUND;
        response.headers_mut().insert(
            header::LOCATION,
            HeaderValue::from_static("/session/magic_link"),
        );
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        *response.body_mut() = Body::empty();
    }
    Ok(response)
}
async fn request_magic_link_inner(
    app: &App,
    peer: SocketAddr,
    headers: HeaderMap,
    body: Value,
    forced_signup: bool,
) -> Result<Response> {
    let email = required(&body, "email_address")?.trim().to_lowercase();
    let guard = app.auth_lock.lock().await;
    auth_attempt(
        app,
        &format!(
            "{}:{}",
            if forced_signup { "signup" } else { "request" },
            peer.ip()
        ),
        10,
        3,
        "Try again later.",
    )
    .await?;
    let existing: Option<String> =
        sqlx::query_scalar("SELECT id FROM identities WHERE email_address=?")
            .bind(&email)
            .fetch_optional(&app.db)
            .await
            .map_err(db_err)?;
    let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let accepting = env::var("RIZZY_MULTI_TENANT").as_deref() == Ok("1") || accounts == 0;
    if forced_signup && !accepting {
        return Err(err(StatusCode::FORBIDDEN, "Signup is closed"));
    }
    let identity_id = if let Some(identity_id) = existing {
        Some((
            identity_id,
            if forced_signup { "sign_up" } else { "sign_in" },
        ))
    } else if accepting {
        if !valid_email(&email) {
            return Err(err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Something went wrong",
            ));
        }
        let identity_id = id();
        sqlx::query("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)")
            .bind(&identity_id)
            .bind(&email)
            .bind(now())
            .execute(&app.db)
            .await
            .map_err(db_err)?;
        Some((identity_id, "sign_up"))
    } else {
        None
    };
    let expires = future_minutes(15);
    let pending = random_token();
    sqlx::query("INSERT INTO pending_authentications(token_hash,email_address,expires_at,created_at) VALUES(?,?,?,?)")
        .bind(hash(&pending)).bind(&email).bind(&expires).bind(now()).execute(&app.db).await.map_err(db_err)?;
    let mut code_header = None;
    if let Some((identity_id, purpose)) = identity_id {
        let code = loop {
            let candidate = magic_code();
            let inserted=sqlx::query("INSERT OR IGNORE INTO magic_links(id,identity_id,code,purpose,expires_at,created_at) VALUES(?,?,?,?,?,?)")
                .bind(id()).bind(&identity_id).bind(&candidate).bind(purpose).bind(&expires).bind(now())
                .execute(&app.db).await.map_err(db_err)?;
            if inserted.rows_affected() == 1 {
                break candidate;
            }
        };
        drop(guard);
        if let Err(error) = deliver_magic_link(&email, &code, &base(&headers)).await {
            sqlx::query("DELETE FROM magic_links WHERE code=?")
                .bind(&code)
                .execute(&app.db)
                .await
                .map_err(db_err)?;
            sqlx::query("DELETE FROM pending_authentications WHERE token_hash=?")
                .bind(hash(&pending))
                .execute(&app.db)
                .await
                .map_err(db_err)?;
            return Err(error);
        }
        if dev_magic_links() {
            code_header = Some(code);
        }
    } else {
        drop(guard);
    }
    let mut response = (
        StatusCode::CREATED,
        Json(json!({"pending_authentication_token":pending})),
    )
        .into_response();
    set_cookie(
        response.headers_mut(),
        "pending_authentication_token",
        &pending,
        900,
        secure_request(&headers),
    );
    if let Some(code) = code_header {
        response.headers_mut().insert(
            "x-magic-link-code",
            HeaderValue::from_str(&code).expect("base32"),
        );
    }
    Ok(response)
}
async fn complete_magic_link(
    State(app): Shared,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let (body, form) = session_body(&headers, &body)?;
    let pending = cookie_value(&headers, "pending_authentication_token").ok_or_else(|| {
        err(
            StatusCode::UNAUTHORIZED,
            "Enter your email address to sign in.",
        )
    })?;
    let _guard = app.auth_lock.lock().await;
    let record = sqlx::query(
        "SELECT email_address FROM pending_authentications WHERE token_hash=? AND expires_at>?",
    )
    .bind(hash(&pending))
    .bind(now())
    .fetch_optional(&app.db)
    .await
    .map_err(db_err)?
    .ok_or_else(|| {
        err(
            StatusCode::UNAUTHORIZED,
            "Enter your email address to sign in.",
        )
    })?;
    auth_attempt(
        &app,
        &format!("verify:{}", peer.ip()),
        10,
        15,
        "Try again in 15 minutes.",
    )
    .await?;
    let code = normalize_magic_code(&required(&body, "code")?);
    let link=sqlx::query("SELECT m.id,m.identity_id,m.purpose,i.email_address FROM magic_links m JOIN identities i ON i.id=m.identity_id WHERE m.code=? AND m.expires_at>?")
        .bind(&code).bind(now()).fetch_optional(&app.db).await.map_err(db_err)?
        .ok_or_else(||err(StatusCode::UNAUTHORIZED,"Try another code."))?;
    let mut response;
    if link.get::<String, _>("email_address") != record.get::<String, _>("email_address") {
        sqlx::query("DELETE FROM magic_links WHERE id=?")
            .bind(link.get::<String, _>("id"))
            .execute(&app.db)
            .await
            .map_err(db_err)?;
        sqlx::query("DELETE FROM pending_authentications WHERE token_hash=?")
            .bind(hash(&pending))
            .execute(&app.db)
            .await
            .map_err(db_err)?;
        response = err(
            StatusCode::UNAUTHORIZED,
            "Something went wrong. Please try again.",
        )
        .into_response();
        set_cookie(
            response.headers_mut(),
            "pending_authentication_token",
            "",
            0,
            secure_request(&headers),
        );
        return Ok(response);
    }
    let token = random_token();
    let purpose: String = link.get("purpose");
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("DELETE FROM magic_links WHERE id=?")
        .bind(link.get::<String, _>("id"))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("DELETE FROM pending_authentications WHERE token_hash=?")
        .bind(hash(&pending))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("INSERT INTO sessions(id,identity_id,token_hash,user_agent,signup_pending,created_at) VALUES(?,?,?,?,?,?)")
        .bind(id()).bind(link.get::<String,_>("identity_id")).bind(hash(&token))
        .bind(headers.get(header::USER_AGENT).and_then(|v|v.to_str().ok()))
        .bind((purpose=="sign_up") as i64).bind(now()).execute(&mut *tx).await.map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    response = Json(json!({"session_token":token,"requires_signup_completion":purpose=="sign_up"}))
        .into_response();
    set_cookie(
        response.headers_mut(),
        "pending_authentication_token",
        "",
        0,
        secure_request(&headers),
    );
    set_cookie(
        response.headers_mut(),
        "session_token",
        &token,
        31536000,
        secure_request(&headers),
    );
    if form {
        *response.status_mut() = StatusCode::FOUND;
        let destination = if purpose == "sign_up" {
            "/signup/completion/new".into()
        } else {
            join_return_to(&headers).unwrap_or_else(|| "/".into())
        };
        response.headers_mut().insert(
            header::LOCATION,
            HeaderValue::from_str(&destination).expect("safe join destination"),
        );
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        *response.body_mut() = Body::empty();
    }
    if cookie_value(&headers, "join_return_to").is_some() {
        set_cookie(
            response.headers_mut(),
            "join_return_to",
            "",
            0,
            secure_request(&headers),
        );
    }
    Ok(response)
}
async fn logout_session(State(app): Shared, headers: HeaderMap) -> Result<Response> {
    let token = cookie_value(&headers, "session_token")
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Authentication required"))?;
    let done = sqlx::query("DELETE FROM sessions WHERE token_hash=?")
        .bind(hash(&token))
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    if done.rows_affected() == 0 {
        return Err(err(StatusCode::UNAUTHORIZED, "Authentication required"));
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    set_cookie(
        response.headers_mut(),
        "session_token",
        "",
        0,
        secure_request(&headers),
    );
    Ok(response)
}
async fn complete_signup(State(app): Shared, headers: HeaderMap, body: Bytes) -> Result<Response> {
    let (identity_id, permission) = identity_auth(&app, &headers).await?;
    if permission != "write" {
        return Err(err(StatusCode::FORBIDDEN, "Write permission required"));
    }
    let (body, form) = session_body(&headers, &body)?;
    let name = if form {
        string(&body, "signup[full_name]").unwrap_or_default()
    } else {
        string(nested(&body, "signup"), "full_name").unwrap_or_default()
    };
    let validation_error = if name.trim().is_empty() {
        Some("Full name can't be blank")
    } else if name.chars().count() > 240 {
        Some("Full name is too long (maximum is 240 characters)")
    } else {
        None
    };
    if let Some(message) = validation_error {
        return Ok(if form {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                signup_page::completion(Some(message)),
            )
                .into_response()
        } else {
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"errors":[message]})),
            )
                .into_response()
        });
    }
    let _writer = app.write_lock.lock().await;
    let mut tx = app.db.begin().await.map_err(db_err)?;
    if let Some(token) = cookie_value(&headers, "session_token") {
        sqlx::query("UPDATE sessions SET signup_pending=0 WHERE token_hash=? AND identity_id=?")
            .bind(hash(&token))
            .bind(&identity_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    let slug: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(external_account_id),0)+1 FROM accounts")
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
    let account_id = id();
    let user_id = id();
    let system_identity = id();
    let system_id = id();
    let timestamp = now();
    let first = name.split_whitespace().next().unwrap_or("New");
    let account_name = format!("{first}'s Rizzy");
    sqlx::query("INSERT INTO accounts(id,external_account_id,name,created_at) VALUES(?,?,?,?)")
        .bind(&account_id)
        .bind(slug)
        .bind(account_name)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    insert_join_code_tx(&mut tx, &account_id)
        .await
        .map_err(db_err)?;
    sqlx::query("INSERT INTO users(id,account_id,identity_id,name,role,created_at,verified_at) VALUES(?,?,?,?,'owner',?,?)")
        .bind(&user_id).bind(&account_id).bind(&identity_id)
        .bind(&name).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
    sqlx::query("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)")
        .bind(&system_identity)
        .bind(format!("system+{account_id}@rizzy.invalid"))
        .bind(&timestamp)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES(?,?,?,'System','system',?)")
        .bind(&system_id).bind(&account_id).bind(&system_identity).bind(&timestamp)
        .execute(&mut *tx).await.map_err(db_err)?;
    signup_seed::playground(&mut tx, &account_id, &user_id, &timestamp)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(if form {
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{slug}/landing"))],
        )
            .into_response()
    } else {
        StatusCode::CREATED.into_response()
    })
}

async fn identity_auth(app: &App, headers: &HeaderMap) -> Result<(String, String)> {
    if let Some(token) = cookie_value(headers, "session_token") {
        let identity_id: Option<String> =
            sqlx::query_scalar("SELECT identity_id FROM sessions WHERE token_hash=?")
                .bind(hash(&token))
                .fetch_optional(&app.db)
                .await
                .map_err(db_err)?;
        if let Some(identity_id) = identity_id {
            return Ok((identity_id, "write".into()));
        }
    }
    let authorization = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    let token = authorization.strip_prefix("Bearer ").ok_or_else(|| {
        if authorization.contains("Bearer") {
            token_denied()
        } else {
            ApiError(StatusCode::FOUND, format!("{}/session/new", base(headers)))
        }
    })?;
    if !headers.contains_key("x-rizzy-json-request") {
        return Err(token_denied());
    }
    let r = sqlx::query("SELECT identity_id,permission FROM access_tokens WHERE token_hash=?")
        .bind(hash(token))
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(token_denied)?;
    if r.get::<String, _>("permission") == "read" && headers.contains_key("x-rizzy-write-request") {
        return Err(token_denied());
    }
    Ok((r.get("identity_id"), r.get("permission")))
}
async fn new_session(State(app): Shared, headers: HeaderMap) -> Result<Response> {
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if accept.contains("application/json") {
        return Ok((
            StatusCode::NOT_ACCEPTABLE,
            Json(json!({"status":406,"error":"Not Acceptable"})),
        )
            .into_response());
    }
    if accept.contains("text/vnd.turbo-stream.html") {
        return Ok(StatusCode::NOT_ACCEPTABLE.into_response());
    }
    let accounts: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM accounts")
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let signup_link = if accounts == 0 || env::var("RIZZY_MULTI_TENANT").as_deref() == Ok("1") {
        "<p><strong>New here?</strong> <a href=\"/signup/new\">Sign up</a> to create an account.</p>"
    } else {
        ""
    };
    let passkey_button = passkeys::sign_in_button().unwrap_or_default();
    Ok(Html(format!("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Enter your email | Rizzy</title></head><body><main><h1>Get into Rizzy</h1><form method=\"post\" action=\"/session\"><label>Enter your email address<input type=\"email\" name=\"email_address\" autocomplete=\"username webauthn\" required autofocus></label><button type=\"submit\">Let's go</button></form>{passkey_button}{signup_link}</main></body></html>")).into_response())
}
async fn check_my_scope(app: &App, identity_id: &str, uri: &Uri) -> Result<()> {
    let mut parts = uri.path().trim_start_matches('/').split('/');
    if let Ok(slug) = parts.next().unwrap_or("").parse::<i64>() {
        if parts.next() == Some("my") {
            let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM users u JOIN accounts a ON a.id=u.account_id WHERE u.identity_id=? AND a.external_account_id=? AND u.active=1 AND NOT EXISTS(SELECT 1 FROM account_cancellations c WHERE c.account_id=a.id)")
                .bind(identity_id).bind(slug).fetch_one(&app.db).await.map_err(db_err)?;
            if count == 0 {
                return Err(err(StatusCode::FORBIDDEN, "Account access required"));
            }
        }
    }
    Ok(())
}
fn config_response(body: &'static str) -> Response {
    (
        [
            (header::CONTENT_TYPE, "application/json"),
            (header::CACHE_CONTROL, "public, max-age=60"),
        ],
        body,
    )
        .into_response()
}
async fn lexxy_asset(Path(filename): Path<String>) -> Result<Response> {
    let (content_type, body): (&str, &'static [u8]) = match filename.as_str() {
        "lexxy.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../static/lexxy/lexxy.js"),
        ),
        "activestorage.esm.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../static/lexxy/activestorage.esm.js"),
        ),
        "lexxy.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../static/lexxy/lexxy.css"),
        ),
        "lexxy-content.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../static/lexxy/lexxy-content.css"),
        ),
        "lexxy-editor.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../static/lexxy/lexxy-editor.css"),
        ),
        "lexxy-variables.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../static/lexxy/lexxy-variables.css"),
        ),
        "rizzy-dark.css" => (
            "text/css; charset=utf-8",
            include_bytes!("../static/lexxy/rizzy-dark.css"),
        ),
        _ => return Err(missing()),
    };
    Ok((
        [(header::CONTENT_TYPE, content_type)],
        Bytes::from_static(body),
    )
        .into_response())
}
async fn android_config() -> Response {
    config_response(include_str!(
        "../static/client_configurations/android_v1.json"
    ))
}
async fn ios_config() -> Response {
    config_response(include_str!("../static/client_configurations/ios_v1.json"))
}
async fn update_timezone(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let timezone = body.get("timezone_name").and_then(Value::as_str);
    sqlx::query("INSERT INTO user_timezones(user_id,account_id,timezone_name) VALUES(?,?,?) ON CONFLICT(user_id) DO UPDATE SET timezone_name=excluded.timezone_name")
        .bind(&p.user_id).bind(&p.account_id).bind(timezone).execute(&app.db).await.map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn my_identity(
    State(app): Shared,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Value>> {
    let (identity_id, _) = identity_auth(&app, &headers).await?;
    check_my_scope(&app, &identity_id, &uri).await?;
    let rows=sqlx::query("SELECT a.id account_id,a.external_account_id,a.name,a.created_at,u.id user_id,u.role FROM accounts a JOIN users u ON u.account_id=a.id WHERE u.identity_id=? AND u.active=1 AND u.role!='system' AND NOT EXISTS(SELECT 1 FROM account_cancellations c WHERE c.account_id=a.id) ORDER BY a.created_at")
        .bind(&identity_id).fetch_all(&app.db).await.map_err(db_err)?;
    let host = base(&headers);
    let mut accounts = Vec::new();
    for r in rows {
        let slug: i64 = r.get("external_account_id");
        let p = Principal {
            account_id: r.get("account_id"),
            user_id: r.get("user_id"),
            role: r.get("role"),
            permission: "read".into(),
            slug,
        };
        accounts.push(json!({"id":p.account_id,"name":r.get::<String,_>("name"),"slug":format!("/{slug}"),"created_at":r.get::<String,_>("created_at"),"user":user_json(&app,&p,&p.user_id,&host).await?}));
    }
    Ok(Json(json!({"id":identity_id,"accounts":accounts})))
}
async fn list_tokens(
    State(app): Shared,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let (identity_id, _) = identity_auth(&app, &headers).await?;
    check_my_scope(&app, &identity_id, &uri).await?;
    let rows=sqlx::query("SELECT id,description,permission,created_at FROM access_tokens WHERE identity_id=? ORDER BY created_at DESC,id DESC")
        .bind(&identity_id).fetch_all(&app.db).await.map_err(db_err)?;
    if headers.contains_key("x-rizzy-json-request") {
        Ok(Json(json!(rows.iter().map(|r|json!({"id":r.get::<String,_>("id"),"description":r.get::<String,_>("description"),"permission":r.get::<String,_>("permission"),"created_at":r.get::<String,_>("created_at")})).collect::<Vec<_>>())).into_response())
    } else {
        let root = token_root(&uri);
        let back = if root.is_empty() {
            "/session/menu".to_owned()
        } else {
            let slug: i64 = root
                .trim_start_matches('/')
                .parse()
                .map_err(|_| missing())?;
            let own_user: Option<String> = sqlx::query_scalar("SELECT u.id FROM users u JOIN accounts a ON a.id=u.account_id WHERE u.identity_id=? AND a.external_account_id=? AND u.active=1")
                .bind(identity_id).bind(slug).fetch_optional(&app.db).await.map_err(db_err)?;
            own_user
                .map(|id| format!("{root}/users/{id}"))
                .unwrap_or_else(|| "/session/menu".into())
        };
        Ok(access_tokens_page::index(&rows, &root, &back).into_response())
    }
}
fn token_root(uri: &Uri) -> String {
    uri.path()
        .trim_start_matches('/')
        .split('/')
        .next()
        .filter(|segment| !segment.is_empty() && segment.bytes().all(|byte| byte.is_ascii_digit()))
        .map(|segment| format!("/{segment}"))
        .unwrap_or_default()
}
async fn new_token(
    State(app): Shared,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let (identity_id, _) = identity_auth(&app, &headers).await?;
    check_my_scope(&app, &identity_id, &uri).await?;
    Ok(access_tokens_page::new(&token_root(&uri)).into_response())
}
async fn show_token(
    State(app): Shared,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let (identity_id, _) = identity_auth(&app, &headers).await?;
    check_my_scope(&app, &identity_id, &uri).await?;
    let reveal_id = uri.path().rsplit('/').next().ok_or_else(missing)?;
    let row = sqlx::query("SELECT a.description,a.permission,r.token,r.identity_id,r.expires_at FROM access_token_reveals r JOIN access_tokens a ON a.id=r.access_token_id WHERE r.id=?")
        .bind(reveal_id).fetch_optional(&app.db).await.map_err(db_err)?;
    let Some(row) = row else {
        return Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("{}/my/access_tokens", token_root(&uri)),
            )],
        )
            .into_response());
    };
    if row.get::<String, _>("identity_id") != identity_id {
        return Err(missing());
    }
    if row.get::<String, _>("expires_at") <= now() {
        sqlx::query("DELETE FROM access_token_reveals WHERE id=?")
            .bind(reveal_id)
            .execute(&app.db)
            .await
            .map_err(db_err)?;
        return Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("{}/my/access_tokens", token_root(&uri)),
            )],
        )
            .into_response());
    }
    let mut response = access_tokens_page::show(
        &token_root(&uri),
        &row.get::<String, _>("description"),
        &row.get::<String, _>("permission"),
        &row.get::<String, _>("token"),
    )
    .into_response();
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}
async fn create_token(
    State(app): Shared,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response> {
    let (identity_id, permission) = identity_auth(&app, &headers).await?;
    check_my_scope(&app, &identity_id, &uri).await?;
    if permission != "write" {
        return Err(err(StatusCode::FORBIDDEN, "Write permission required"));
    }
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded");
    let body: Value = if form {
        let fields: HashMap<String, String> = url::form_urlencoded::parse(&body)
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        json!({"access_token":{"description":fields.get("access_token[description]").cloned().unwrap_or_default(),"permission":fields.get("access_token[permission]").cloned().unwrap_or_else(||"read".into())}})
    } else {
        serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?
    };
    let v = nested(&body, "access_token");
    let description = string(v, "description").unwrap_or_default();
    let token_permission = string(v, "permission").unwrap_or_else(|| "read".into());
    if token_permission != "read" && token_permission != "write" {
        return Err(bad("invalid permission"));
    }
    let id = id();
    let token = hex::encode(rand::random::<[u8; 32]>());
    let created = now();
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES(?,?,?,?,?,?)")
        .bind(&id).bind(&identity_id).bind(hash(&token)).bind(&description).bind(&token_permission).bind(&created).execute(&mut *tx).await.map_err(db_err)?;
    let reveal_id = if form {
        let reveal_id = random_token();
        let expires = (Utc::now() + chrono::Duration::seconds(10))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        sqlx::query("INSERT INTO access_token_reveals(id,access_token_id,identity_id,token,expires_at) VALUES(?,?,?,?,?)")
            .bind(&reveal_id).bind(&id).bind(&identity_id).bind(&token).bind(expires).execute(&mut *tx).await.map_err(db_err)?;
        Some(reveal_id)
    } else {
        None
    };
    tx.commit().await.map_err(db_err)?;
    if let Some(reveal_id) = reveal_id {
        Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("{}/my/access_tokens/{reveal_id}", token_root(&uri)),
            )],
        )
            .into_response())
    } else {
        Ok((StatusCode::CREATED,Json(json!({"id":id,"token":token,"description":description,"permission":token_permission,"created_at":created}))).into_response())
    }
}
async fn delete_token(
    State(app): Shared,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    method: Method,
    body: Bytes,
) -> Result<Response> {
    let (identity_id, permission) = identity_auth(&app, &headers).await?;
    check_my_scope(&app, &identity_id, &uri).await?;
    if method == Method::POST
        && (!headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .starts_with("application/x-www-form-urlencoded")
            || !url::form_urlencoded::parse(&body).any(|(k, v)| k == "_method" && v == "delete"))
    {
        return Err(missing());
    }
    if permission != "write" {
        return Err(err(StatusCode::FORBIDDEN, "Write permission required"));
    }
    let id = uri.path().rsplit('/').next().ok_or_else(missing)?;
    let done = sqlx::query("DELETE FROM access_tokens WHERE id=? AND identity_id=?")
        .bind(id)
        .bind(identity_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    if done.rows_affected() == 0 {
        return Err(missing());
    }
    Ok(if headers.contains_key("x-rizzy-json-request") {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("{}/my/access_tokens", token_root(&uri)),
            )],
        )
            .into_response()
    })
}
async fn active_user(app: &App, p: &Principal, id: &str) -> Result<SqliteRow> {
    sqlx::query("SELECT * FROM users WHERE id=? AND account_id=? AND active=1 AND role!='system'")
        .bind(id.trim_end_matches(".json"))
        .bind(&p.account_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)
}
fn prompt_response(body: String, headers: &HeaderMap) -> Response {
    let etag = format!("W/\"{}\"", hash(&body));
    let not_modified = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag));
    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Html(body).into_response()
    };
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("hex ETag"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=0, private, must-revalidate"),
    );
    response
}

fn prompt_user_item(p: &Principal, row: &SqliteRow) -> String {
    let id: String = row.get("id");
    let name: String = row.get("name");
    let sgid: String = row.get("sgid");
    let first = name.split_whitespace().next().unwrap_or(&name);
    let initials: String = name
        .split_whitespace()
        .filter_map(|word| word.chars().next())
        .flat_map(char::to_uppercase)
        .collect();
    let self_label = if id == p.user_id { " me" } else { "" };
    let avatar = format!(
        "<img aria-hidden=\"true\" title=\"{}\" src=\"/{}/users/{id}/avatar\" width=\"48\" height=\"48\" />",
        escape(&name),
        p.slug
    );
    format!(
        "<lexxy-prompt-item search=\"{} {}{}\" sgid=\"{}\">\n  <template type=\"menu\">\n    {}\n    {}\n  </template>\n  <template type=\"editor\">\n    {}\n    {}\n  </template>\n</lexxy-prompt-item>\n\n",
        escape(&name),
        escape(&initials),
        self_label,
        escape(&sgid),
        avatar,
        escape(&name),
        avatar,
        escape(first)
    )
}

async fn prompt_users(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let rows = sqlx::query("SELECT u.id,u.name,s.sgid FROM users u JOIN user_attachables s ON s.user_id=u.id WHERE u.account_id=? AND u.active=1 AND u.role!='system' ORDER BY lower(u.name),u.id")
        .bind(&p.account_id).fetch_all(&app.db).await.map_err(db_err)?;
    Ok(prompt_response(
        rows.iter().map(|row| prompt_user_item(&p, row)).collect(),
        &headers,
    ))
}

async fn prompt_board_users(
    State(app): Shared,
    Path((account, board_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    board(&app, &p, &board_id).await?;
    let rows = sqlx::query("SELECT u.id,u.name,s.sgid FROM users u JOIN user_attachables s ON s.user_id=u.id JOIN accesses a ON a.user_id=u.id WHERE u.account_id=? AND a.board_id=? AND u.active=1 AND u.role!='system' ORDER BY lower(u.name),u.id")
        .bind(&p.account_id).bind(&board_id).fetch_all(&app.db).await.map_err(db_err)?;
    Ok(prompt_response(
        rows.iter().map(|row| prompt_user_item(&p, row)).collect(),
        &headers,
    ))
}

#[derive(Deserialize)]
struct PromptCardQuery {
    filter: Option<String>,
}

fn prompt_card_item(slug: i64, row: &SqliteRow) -> String {
    let number: i64 = row.get("number");
    let title: String = row.get("title");
    let label = format!("#{number} {}", escape(&title));
    format!(
        "<lexxy-prompt-item>\n  <template type=\"menu\">\n    {label}\n  </template>\n  <template type=\"editor\">\n    <a href=\"/{slug}/cards/{number}\">{label}</a>\n  </template>\n</lexxy-prompt-item>\n\n"
    )
}

async fn prompt_cards(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Query(query): Query<PromptCardQuery>,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let mut rows = Vec::new();
    if let Some(filter) = query
        .filter
        .as_deref()
        .filter(|value| !value.trim().is_empty())
    {
        if let Ok(number) = filter.parse::<i64>() {
            if let Some(exact) = sqlx::query("SELECT c.number,c.title FROM cards c WHERE c.account_id=? AND c.number=? AND EXISTS(SELECT 1 FROM accesses a WHERE a.board_id=c.board_id AND a.user_id=?)")
                .bind(&p.account_id).bind(number).bind(&p.user_id)
                .fetch_optional(&app.db).await.map_err(db_err)? {
                rows.push(exact);
            }
        }
        if let Some(search) = fts_query(filter) {
            let matched = sqlx::query("SELECT c.number,c.title FROM cards c JOIN (SELECT id AS card_id FROM cards WHERE rowid IN (SELECT rowid FROM card_search WHERE card_search MATCH ?) UNION ALL SELECT card_id FROM comments WHERE rowid IN (SELECT rowid FROM comment_search WHERE comment_search MATCH ?)) hits ON hits.card_id=c.id WHERE c.account_id=? AND c.status='published' AND EXISTS(SELECT 1 FROM accesses a WHERE a.board_id=c.board_id AND a.user_id=?) ORDER BY c.created_at DESC,c.id DESC LIMIT 10")
                .bind(&search).bind(&search).bind(&p.account_id).bind(&p.user_id)
                .fetch_all(&app.db).await.map_err(db_err)?;
            rows.extend(matched);
        }
    } else {
        rows = sqlx::query("SELECT c.number,c.title FROM cards c WHERE c.account_id=? AND c.status='published' AND EXISTS(SELECT 1 FROM accesses a WHERE a.board_id=c.board_id AND a.user_id=?) ORDER BY c.last_active_at DESC,c.id DESC")
            .bind(&p.account_id).bind(&p.user_id).fetch_all(&app.db).await.map_err(db_err)?;
    }
    let html: String = rows
        .iter()
        .map(|row| prompt_card_item(p.slug, row))
        .collect();
    Ok(prompt_response(html, &headers))
}
async fn list_users(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let scope = " FROM users WHERE account_id=? AND active=1 AND role!='system'";
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*){scope}"))
        .bind(&p.account_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let rows = sqlx::query(&format!(
        "SELECT id{scope} ORDER BY lower(name),id LIMIT ? OFFSET ?"
    ))
    .bind(&p.account_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let host = base(&headers);
    let mut out = Vec::new();
    for r in rows {
        out.push(user_json(&app, &p, &r.get::<String, _>("id"), &host).await?);
    }
    Ok(paginated(json!(out), total, page, &uri, &headers))
}
async fn show_user(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let u = active_user(&app, &p, &id).await?;
    if headers.contains_key("x-rizzy-json-request") {
        Ok(
            Json(user_json(&app, &p, &u.get::<String, _>("id"), &base(&headers)).await?)
                .into_response(),
        )
    } else {
        Ok(user_page::show(&app, &p, &u, &headers)
            .await?
            .into_response())
    }
}
async fn edit_user(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Html<String>> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let p = principal(&app, &headers, &account).await?;
    let u = active_user(&app, &p, &id).await?;
    let email: String = sqlx::query_scalar("SELECT email_address FROM identities WHERE id=?")
        .bind(u.get::<String, _>("identity_id"))
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    Ok(user_page::edit(&p, &u, &email, None))
}
fn can_change_user(p: &Principal, u: &SqliteRow) -> Result<()> {
    if p.user_id == u.get::<String, _>("id")
        || ((p.role == "owner" || p.role == "admin") && u.get::<String, _>("role") != "owner")
    {
        Ok(())
    } else {
        Err(err(StatusCode::FORBIDDEN, "Cannot change user"))
    }
}
async fn update_user(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Response> {
    let (body, form) = if headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded")
    {
        let mut user = serde_json::Map::new();
        let mut fields = serde_json::Map::new();
        for (key, value) in url::form_urlencoded::parse(&body) {
            match key.as_ref() {
                "user[name]" => {
                    user.insert("name".into(), json!(value.into_owned()));
                }
                "_method" => {
                    fields.insert("_method".into(), json!(value.into_owned()));
                }
                _ => {}
            }
        }
        fields.insert("user".into(), Value::Object(user));
        (Value::Object(fields), true)
    } else {
        (
            serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?,
            false,
        )
    };
    if method == Method::POST {
        match (form, body.get("_method").and_then(Value::as_str)) {
            (true, Some("delete")) => {
                return delete_user(State(app), Path((account, id)), headers).await;
            }
            (true, Some("patch")) => {}
            _ => return Err(missing()),
        }
    }
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let u = active_user(&app, &p, &id).await?;
    can_change_user(&p, &u)?;
    let values = nested(&body, "user");
    let name = string(values, "name").unwrap_or_else(|| u.get("name"));
    if name.trim().is_empty() {
        if form {
            let email: String =
                sqlx::query_scalar("SELECT email_address FROM identities WHERE id=?")
                    .bind(u.get::<String, _>("identity_id"))
                    .fetch_one(&app.db)
                    .await
                    .map_err(db_err)?;
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                user_page::edit(&p, &u, &email, Some("Name can't be blank")),
            )
                .into_response());
        }
        return Err(bad("name is required"));
    }
    let avatar = if let Some(value) = values.get("avatar") {
        Some(storage::prepare_user_avatar(&app, &p, value).await?)
    } else {
        None
    };
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("UPDATE users SET name=? WHERE id=?")
        .bind(name)
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    let removed = if let Some(blob_id) = avatar {
        storage::attach_user_avatar_tx(&mut tx, &p, &id, blob_id.as_deref()).await?
    } else {
        Vec::new()
    };
    tx.commit().await.map_err(db_err)?;
    storage::remove_files(removed).await;
    if form {
        Ok((
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/users/{id}"))],
        )
            .into_response())
    } else {
        Ok(Json(user_json(&app, &p, &id, &base(&headers)).await?).into_response())
    }
}
async fn delete_user(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let u = active_user(&app, &p, &id).await?;
    can_change_user(&p, &u)?;
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("DELETE FROM accesses WHERE user_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("UPDATE users SET active=0,identity_id=NULL WHERE id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(if headers.contains_key("x-rizzy-json-request") {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/account/settings"))],
        )
            .into_response()
    })
}
async fn update_user_role(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Response> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded");
    let body: Value = if form {
        let fields: HashMap<String, String> = url::form_urlencoded::parse(&body)
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        if method == Method::POST && fields.get("_method").map(String::as_str) != Some("patch") {
            return Err(missing());
        }
        json!({"user":{"role":fields.get("user[role]").cloned().unwrap_or_default()}})
    } else {
        if method == Method::POST {
            return Err(missing());
        }
        serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?
    };
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let user = active_user(&app, &p, &id).await?;
    if !matches!(p.role.as_str(), "owner" | "admin")
        || p.user_id == id
        || user.get::<String, _>("role") == "owner"
    {
        return Err(err(StatusCode::FORBIDDEN, "Cannot administer user"));
    }
    let role = string(nested(&body, "user"), "role").unwrap_or_default();
    let role = if role == "admin" { "admin" } else { "member" };
    sqlx::query("UPDATE users SET role=? WHERE id=?")
        .bind(role)
        .bind(&id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(if form {
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/account/settings"))],
        )
            .into_response()
    } else {
        StatusCode::NO_CONTENT.into_response()
    })
}
fn email_change_invalid(headers: &HeaderMap, account: i64, user_id: &str) -> Response {
    if headers.contains_key("x-rizzy-json-request")
        || headers
            .get(header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("json"))
    {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":"Token is invalid or has expired"})),
        )
            .into_response()
    } else {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            user_page::expired_email(account, user_id),
        )
            .into_response()
    }
}
async fn deliver_email_change(email: &str, link: &str) -> Result<()> {
    if dev_magic_links() {
        return Ok(());
    }
    let path = env::var("RIZZY_SENDMAIL").map_err(|_| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "Mail delivery is not configured",
        )
    })?;
    let from = env::var("RIZZY_MAIL_FROM").unwrap_or_else(|_| "no-reply@localhost".into());
    if !valid_email(email) || from.contains(['\r', '\n']) {
        return Err(bad("invalid email address"));
    }
    let message = format!(
        "From: {from}\nTo: {email}\nSubject: Confirm your Rizzy email address\nContent-Type: text/plain; charset=UTF-8\n\nConfirm your new email address within 30 minutes:\n{link}\n"
    );
    let mut child = Command::new(path)
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
    let status = child
        .wait()
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"))?;
    if !status.success() {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"));
    }
    Ok(())
}
async fn new_email_change(
    State(app): Shared,
    Path((account, user_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Html<String>> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let p = principal(&app, &headers, &account).await?;
    if p.user_id != user_id {
        return Err(missing());
    }
    active_user(&app, &p, &user_id).await?;
    Ok(user_page::new_email(&p, &user_id))
}
fn email_change_error(form: bool, p: &Principal, user_id: &str, message: &str) -> Response {
    if form {
        (
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("/{}/users/{user_id}/email_addresses/new", p.slug),
            )],
        )
            .into_response()
    } else {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error":message})),
        )
            .into_response()
    }
}
async fn request_email_change(
    State(app): Shared,
    Path((account, user_id)): Path<(String, String)>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let (body, form) = session_body(&headers, &body)?;
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    if p.user_id != user_id {
        return Err(missing());
    }
    let Some(raw) = body
        .get("email_address")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
    else {
        return Ok(StatusCode::BAD_REQUEST.into_response());
    };
    let email = raw.trim().to_lowercase();
    if !valid_email(&email) {
        return Ok(email_change_error(
            form,
            &p,
            &user_id,
            "Please enter a valid email address",
        ));
    }
    let _guard = app.auth_lock.lock().await;
    auth_attempt(
        &app,
        &format!("email-change:{}", peer.ip()),
        5,
        60,
        "Try again later.",
    )
    .await?;
    let old=sqlx::query("SELECT u.identity_id,i.email_address FROM users u JOIN identities i ON i.id=u.identity_id WHERE u.id=? AND u.account_id=? AND u.active=1")
        .bind(&p.user_id).bind(&p.account_id).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    let old_identity: String = old.get("identity_id");
    if old
        .get::<String, _>("email_address")
        .eq_ignore_ascii_case(&email)
    {
        return Ok(email_change_error(
            form,
            &p,
            &user_id,
            "That is already your email address",
        ));
    }
    let already:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users u JOIN identities i ON i.id=u.identity_id WHERE u.account_id=? AND lower(i.email_address)=?)")
        .bind(&p.account_id).bind(&email).fetch_one(&app.db).await.map_err(db_err)?;
    if already != 0 {
        return Ok(email_change_error(
            form,
            &p,
            &user_id,
            "You already have a user in this account with that email address",
        ));
    }
    let token = random_token();
    sqlx::query("INSERT INTO email_change_requests(token_hash,account_id,user_id,old_identity_id,new_email_address,expires_at,created_at) VALUES(?,?,?,?,?,?,?)")
        .bind(hash(&token)).bind(&p.account_id).bind(&p.user_id).bind(old_identity).bind(&email)
        .bind(future_minutes(30)).bind(now()).execute(&app.db).await.map_err(db_err)?;
    let link = format!(
        "{}/{}/users/{user_id}/email_addresses/{token}/confirmation",
        base(&headers),
        p.slug
    );
    if let Err(error) = deliver_email_change(&email, &link).await {
        sqlx::query("DELETE FROM email_change_requests WHERE token_hash=?")
            .bind(hash(&token))
            .execute(&app.db)
            .await
            .map_err(db_err)?;
        return Err(error);
    }
    let mut response = if form {
        user_page::email_sent(&p, &user_id, &email).into_response()
    } else {
        StatusCode::CREATED.into_response()
    };
    if dev_magic_links() {
        response.headers_mut().insert(
            "x-email-change-token",
            HeaderValue::from_str(&token).expect("hex token"),
        );
    }
    Ok(response)
}
async fn show_email_change_confirmation(
    State(app): Shared,
    Path((account, user_id, token)): Path<(String, String, String)>,
) -> Result<Html<String>> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let exists:i64=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users u JOIN accounts a ON a.id=u.account_id WHERE u.id=? AND u.active=1 AND a.external_account_id=?)")
        .bind(&user_id).bind(slug).fetch_one(&app.db).await.map_err(db_err)?;
    if exists == 0 {
        return Err(missing());
    }
    Ok(user_page::confirm_email(slug, &user_id, &token))
}
async fn confirm_email_change(
    State(app): Shared,
    Path((account, user_id, token)): Path<(String, String, String)>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let user=sqlx::query("SELECT u.id,u.account_id,u.identity_id FROM users u JOIN accounts a ON a.id=u.account_id WHERE u.id=? AND u.active=1 AND a.external_account_id=?")
        .bind(&user_id).bind(slug).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    let _guard = app.auth_lock.lock().await;
    auth_attempt(
        &app,
        &format!("email-confirm:{}", peer.ip()),
        5,
        60,
        "Try again later.",
    )
    .await?;
    let Some(request)=sqlx::query("SELECT old_identity_id,new_email_address FROM email_change_requests WHERE token_hash=? AND user_id=? AND account_id=? AND expires_at>?")
        .bind(hash(&token)).bind(&user_id).bind(user.get::<String,_>("account_id")).bind(now())
        .fetch_optional(&app.db).await.map_err(db_err)? else {return Ok(email_change_invalid(&headers,slug,&user_id));};
    let old_identity: String = request.get("old_identity_id");
    if old_identity != user.get::<String, _>("identity_id") {
        return Ok(email_change_invalid(&headers, slug, &user_id));
    }
    let new_email: String = request.get("new_email_address");
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("INSERT OR IGNORE INTO identities(id,email_address,created_at) VALUES(?,?,?)")
        .bind(id())
        .bind(&new_email)
        .bind(now())
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    let new_identity: String =
        sqlx::query_scalar("SELECT id FROM identities WHERE email_address=?")
            .bind(&new_email)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
    let changed=sqlx::query("UPDATE OR IGNORE users SET identity_id=? WHERE id=? AND account_id=? AND identity_id=? AND active=1")
        .bind(&new_identity).bind(&user_id).bind(user.get::<String,_>("account_id")).bind(&old_identity)
        .execute(&mut *tx).await.map_err(db_err)?;
    if changed.rows_affected() == 0 {
        return Ok(email_change_invalid(&headers, slug, &user_id));
    }
    sqlx::query("DELETE FROM email_change_requests WHERE user_id=? AND account_id=?")
        .bind(&user_id)
        .bind(user.get::<String, _>("account_id"))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    if let Some(session) = cookie_value(&headers, "session_token") {
        sqlx::query("DELETE FROM sessions WHERE token_hash=?")
            .bind(hash(&session))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    let session = random_token();
    sqlx::query("INSERT INTO sessions(id,identity_id,token_hash,user_agent,signup_pending,created_at) VALUES(?,?,?,?,0,?)")
        .bind(id()).bind(&new_identity).bind(hash(&session))
        .bind(headers.get(header::USER_AGENT).and_then(|v|v.to_str().ok()))
        .bind(now()).execute(&mut *tx).await.map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    let wants_json = headers
        .get(header::ACCEPT)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|accept| accept.contains("json"));
    let mut response = if wants_json {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/users/{user_id}/edit"))],
        )
            .into_response()
    };
    set_cookie(
        response.headers_mut(),
        "session_token",
        &session,
        31536000,
        secure_request(&headers),
    );
    Ok(response)
}
fn valid_entropy(days: i64) -> Result<i64> {
    if [3, 7, 11, 30, 90, 365].contains(&days) {
        Ok(days)
    } else {
        Err(bad("invalid auto_postpone_period_in_days"))
    }
}
async fn account_json(app: &App, p: &Principal) -> Result<Value> {
    let row = sqlx::query("SELECT id,name,cards_count,created_at,auto_postpone_period_in_days FROM accounts WHERE id=?")
        .bind(&p.account_id).fetch_one(&app.db).await.map_err(db_err)?;
    Ok(
        json!({"id":row.get::<String,_>("id"),"name":row.get::<String,_>("name"),
        "cards_count":row.get::<i64,_>("cards_count"),"created_at":row.get::<String,_>("created_at"),
        "auto_postpone_period_in_days":row.get::<i64,_>("auto_postpone_period_in_days")}),
    )
}
async fn show_account(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    if headers.contains_key("x-rizzy-json-request") {
        Ok(Json(account_json(&app, &p).await?).into_response())
    } else {
        Ok(account_settings::show(&app, &p, None)
            .await?
            .into_response())
    }
}
async fn show_join_code(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let row = sqlx::query(
        "SELECT code,usage_count,usage_limit FROM account_join_codes WHERE account_id=?",
    )
    .bind(&p.account_id)
    .fetch_one(&app.db)
    .await
    .map_err(db_err)?;
    let code: String = row.get("code");
    let count: i64 = row.get("usage_count");
    let limit: i64 = row.get("usage_limit");
    let url = format!("{}/{}/join/{}", base(&headers), p.slug, code);
    if headers.contains_key("x-rizzy-json-request") {
        Ok(
            Json(json!({"code":code,"usage_count":count,"usage_limit":limit,
        "url":url,"active":count<limit}))
            .into_response(),
        )
    } else {
        Ok(join_code_page::show(&p, &url, count, limit).into_response())
    }
}
async fn edit_join_code(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Html<String>> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let p = principal(&app, &headers, &account).await?;
    let row =
        sqlx::query("SELECT usage_count,usage_limit FROM account_join_codes WHERE account_id=?")
            .bind(&p.account_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
    Ok(join_code_page::edit(
        &p,
        row.get("usage_count"),
        row.get("usage_limit"),
        None,
    ))
}
async fn update_join_code(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Response> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded");
    let body: Value = if form {
        let fields: HashMap<String, String> = url::form_urlencoded::parse(&body)
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if method == Method::POST && fields.get("_method").map(String::as_str) != Some("patch") {
            return Err(missing());
        }
        json!({"account_join_code":{"usage_limit":fields.get("account_join_code[usage_limit]").cloned().unwrap_or_default()}})
    } else {
        if method == Method::POST {
            return Err(missing());
        }
        serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?
    };
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    p.account_admin()?;
    let v = nested(&body, "account_join_code");
    let limit = v
        .get("usage_limit")
        .and_then(|v| {
            v.as_i64()
                .or_else(|| v.as_str().and_then(|s| s.parse().ok()))
        })
        .ok_or_else(|| bad("usage_limit is invalid"));
    let limit = match limit {
        Ok(limit) => limit,
        Err(_) if form => {
            let row = sqlx::query(
                "SELECT usage_count,usage_limit FROM account_join_codes WHERE account_id=?",
            )
            .bind(&p.account_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
            let message = if v
                .get("usage_limit")
                .and_then(Value::as_str)
                .is_some_and(|raw| raw.len() > 11 && raw.bytes().all(|byte| byte.is_ascii_digit()))
            {
                "Usage limit cannot be larger than the population of the planet"
            } else {
                "Usage limit is invalid"
            };
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                join_code_page::edit(
                    &p,
                    row.get("usage_count"),
                    row.get("usage_limit"),
                    Some(message),
                ),
            )
                .into_response());
        }
        Err(error) => return Err(error),
    };
    if !(0..=10_000_000_000).contains(&limit) {
        if form {
            let row = sqlx::query(
                "SELECT usage_count,usage_limit FROM account_join_codes WHERE account_id=?",
            )
            .bind(&p.account_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                join_code_page::edit(
                    &p,
                    row.get("usage_count"),
                    row.get("usage_limit"),
                    Some("Usage limit cannot be larger than the population of the planet"),
                ),
            )
                .into_response());
        }
        return Err(bad(
            "usage_limit cannot be larger than the population of the planet",
        ));
    }
    sqlx::query("UPDATE account_join_codes SET usage_limit=?,updated_at=? WHERE account_id=?")
        .bind(limit)
        .bind(now())
        .bind(&p.account_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(if form {
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/account/join_code"))],
        )
            .into_response()
    } else {
        StatusCode::NO_CONTENT.into_response()
    })
}
async fn reset_join_code(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    p.account_admin()?;
    loop {
        let result=sqlx::query("UPDATE OR IGNORE account_join_codes SET code=?,usage_count=0,updated_at=? WHERE account_id=?")
            .bind(join_code()).bind(now()).bind(&p.account_id).execute(&app.db).await.map_err(db_err)?;
        if result.rows_affected() == 1 {
            break;
        }
        let exists: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM account_join_codes WHERE account_id=?)",
        )
        .bind(&p.account_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
        if exists == 0 {
            return Err(missing());
        }
    }
    Ok(if headers.contains_key("x-rizzy-json-request") {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/account/join_code"))],
        )
            .into_response()
    })
}
async fn join_page(
    State(app): Shared,
    Path((account, code)): Path<(String, String)>,
) -> Result<Response> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let row=sqlx::query("SELECT a.name,j.usage_count,j.usage_limit FROM account_join_codes j JOIN accounts a ON a.id=j.account_id WHERE a.external_account_id=? AND j.code=?")
        .bind(slug).bind(&code).fetch_optional(&app.db).await.map_err(db_err)?;
    let Some(row) = row else {
        return Ok((
            StatusCode::NOT_FOUND,
            Html("<h1>Invite link not found</h1>"),
        )
            .into_response());
    };
    let name: String = row.get("name");
    if row.get::<i64, _>("usage_count") >= row.get::<i64, _>("usage_limit") {
        return Ok((StatusCode::GONE, join_code_page::inactive(&name)).into_response());
    }
    let page = include_str!("../static/join.html")
        .replace("{{ACCOUNT_NAME}}", &escape(&name))
        .replace("{{ACCOUNT}}", &slug.to_string())
        .replace("{{CODE}}", &escape(&code));
    Ok(Html(page).into_response())
}
async fn redeem_join_code(
    State(app): Shared,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path((account, code)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded");
    let body: Value = if form {
        let email = url::form_urlencoded::parse(&body)
            .find(|(key, _)| key == "email_address")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default();
        json!({"email_address":email})
    } else {
        serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?
    };
    let email = string(&body, "email_address")
        .unwrap_or_default()
        .trim()
        .to_lowercase();
    let existing=sqlx::query("SELECT a.id,j.usage_count,j.usage_limit FROM account_join_codes j JOIN accounts a ON a.id=j.account_id WHERE a.external_account_id=? AND j.code=?")
        .bind(slug).bind(&code).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    if existing.get::<i64, _>("usage_count") >= existing.get::<i64, _>("usage_limit") {
        if form {
            let name: String = sqlx::query_scalar("SELECT name FROM accounts WHERE id=?")
                .bind(existing.get::<String, _>("id"))
                .fetch_one(&app.db)
                .await
                .map_err(db_err)?;
            return Ok((StatusCode::GONE, join_code_page::inactive(&name)).into_response());
        }
        return Err(err(StatusCode::GONE, "That code is all used up"));
    }
    if !valid_email(&email) {
        if form {
            return Ok(StatusCode::UNPROCESSABLE_ENTITY.into_response());
        }
        return Err(bad("Invalid email address"));
    }
    let account_id: String = existing.get("id");
    {
        let _auth_guard = app.auth_lock.lock().await;
        auth_attempt(
            &app,
            &format!("join:{}", peer.ip()),
            10,
            3,
            "Try again later.",
        )
        .await?;
    }
    let current = identity_auth(&app, &headers)
        .await
        .ok()
        .and_then(|(identity, permission)| {
            if permission == "write" {
                Some(identity)
            } else {
                None
            }
        });
    let _writer = app.write_lock.lock().await;
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let claimed=sqlx::query("UPDATE account_join_codes SET usage_count=usage_count+1,updated_at=? WHERE account_id=? AND code=? AND usage_count<usage_limit")
        .bind(now()).bind(&account_id).bind(&code).execute(&mut *tx).await.map_err(db_err)?;
    if claimed.rows_affected() == 0 {
        return Err(err(StatusCode::GONE, "That code is all used up"));
    }
    sqlx::query("INSERT OR IGNORE INTO identities(id,email_address,created_at) VALUES(?,?,?)")
        .bind(id())
        .bind(&email)
        .bind(now())
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    let identity_id: String = sqlx::query_scalar("SELECT id FROM identities WHERE email_address=?")
        .bind(&email)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
    let inserted=sqlx::query("INSERT OR IGNORE INTO users(id,account_id,identity_id,name,role,active,created_at) VALUES(?,?,?,?,'member',1,?)")
        .bind(id()).bind(&account_id).bind(&identity_id).bind(&email).bind(now())
        .execute(&mut *tx).await.map_err(db_err)?;
    if inserted.rows_affected() == 1 {
        sqlx::query("INSERT OR IGNORE INTO accesses(board_id,user_id) SELECT b.id,u.id FROM boards b JOIN users u ON u.account_id=b.account_id WHERE b.account_id=? AND b.all_access=1 AND u.identity_id=?")
            .bind(&account_id).bind(&identity_id).execute(&mut *tx).await.map_err(db_err)?;
    } else {
        sqlx::query("UPDATE account_join_codes SET usage_count=usage_count-1 WHERE account_id=?")
            .bind(&account_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    let member = sqlx::query("SELECT name,active FROM users WHERE account_id=? AND identity_id=?")
        .bind(&account_id)
        .bind(&identity_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
    if member.get::<i64, _>("active") == 0 {
        return Err(missing());
    }
    let requires_name = member.get::<String, _>("name") == email;
    tx.commit().await.map_err(db_err)?;
    drop(_writer);
    if current.as_deref() == Some(identity_id.as_str()) {
        return Ok(if form {
            (
                StatusCode::FOUND,
                [(
                    header::LOCATION,
                    if requires_name {
                        format!("/{slug}/users/verifications/new")
                    } else {
                        format!("/{slug}/landing")
                    },
                )],
            )
                .into_response()
        } else {
            Json(json!({"joined":true,"requires_name":requires_name,"account_slug":format!("/{slug}")})).into_response()
        });
    }
    if let Some(session) = cookie_value(&headers, "session_token") {
        sqlx::query("DELETE FROM sessions WHERE token_hash=?")
            .bind(hash(&session))
            .execute(&app.db)
            .await
            .map_err(db_err)?;
    }
    let mut response = request_magic_link_inner(
        &app,
        peer,
        headers.clone(),
        json!({"email_address":email}),
        false,
    )
    .await?;
    if current.is_some() {
        set_cookie(
            response.headers_mut(),
            "session_token",
            "",
            0,
            secure_request(&headers),
        );
    }
    if form {
        set_cookie(
            response.headers_mut(),
            "join_return_to",
            &format!("/{slug}/users/verifications/new"),
            900,
            secure_request(&headers),
        );
        *response.status_mut() = StatusCode::FOUND;
        response.headers_mut().insert(
            header::LOCATION,
            HeaderValue::from_static("/session/magic_link"),
        );
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/html; charset=utf-8"),
        );
        *response.body_mut() = Body::empty();
    }
    Ok(response)
}
async fn complete_join(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded");
    let body: Value = if form {
        let name = url::form_urlencoded::parse(&body)
            .find(|(key, _)| key == "user[name]")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default();
        json!({"user":{"name":name}})
    } else {
        serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?
    };
    let name = required(nested(&body, "user"), "name")?;
    sqlx::query("UPDATE users SET name=? WHERE id=? AND account_id=?")
        .bind(name)
        .bind(&p.user_id)
        .bind(&p.account_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    if form {
        Ok((
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/landing"))],
        )
            .into_response())
    } else {
        Ok(StatusCode::NO_CONTENT.into_response())
    }
}
async fn new_user_join(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Html<String>> {
    let p = principal(&app, &headers, &account).await?;
    Ok(Html(format!(
        "<!doctype html><html><title>Enter your name</title><main><h1>Great! Now enter your name</h1><form method=post action=\"/{}/users/joins\"><label>Full name <input name=\"user[name]\" autocomplete=name required autofocus></label><button id=log_in>Continue</button></form></main></html>",
        p.slug
    )))
}
async fn new_user_verification(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Html<String>> {
    let p = principal(&app, &headers, &account).await?;
    Ok(Html(format!(
        "<!doctype html><html><title>Verify</title><main><form method=post action=\"/{}/users/verifications\"><button>Continue</button></form></main><script>document.forms[0].requestSubmit()</script></html>",
        p.slug
    )))
}
async fn verify_user(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    sqlx::query("UPDATE users SET verified_at=COALESCE(verified_at,?) WHERE id=? AND account_id=?")
        .bind(now())
        .bind(&p.user_id)
        .bind(&p.account_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, format!("/{}/users/joins/new", p.slug))],
    )
        .into_response())
}
async fn update_account(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Response> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded");
    let body: Value = if form {
        let fields: HashMap<String, String> = url::form_urlencoded::parse(&body)
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if method == Method::POST && fields.get("_method").map(String::as_str) != Some("put") {
            return Err(missing());
        }
        json!({"account":{"name":fields.get("account[name]").cloned().unwrap_or_default()}})
    } else {
        if method == Method::POST {
            return Err(missing());
        }
        serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?
    };
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    p.account_admin()?;
    let name = string(nested(&body, "account"), "name").unwrap_or_default();
    if name.trim().is_empty() {
        if form {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                account_settings::show(&app, &p, Some("Name can't be blank")).await?,
            )
                .into_response());
        }
        return Err(bad("name is required"));
    }
    sqlx::query("UPDATE accounts SET name=? WHERE id=?")
        .bind(name)
        .bind(&p.account_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(if form {
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/account/settings"))],
        )
            .into_response()
    } else {
        StatusCode::NO_CONTENT.into_response()
    })
}
async fn update_account_entropy(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Response> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded");
    let body: Value = if form {
        let fields: HashMap<String, String> = url::form_urlencoded::parse(&body)
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        if method == Method::POST && fields.get("_method").map(String::as_str) != Some("put") {
            return Err(missing());
        }
        json!({"entropy":{"auto_postpone_period_in_days":fields.get("entropy[auto_postpone_period_in_days]").and_then(|value| value.parse::<i64>().ok())}})
    } else {
        if method == Method::POST {
            return Err(missing());
        }
        serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?
    };
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    p.account_admin()?;
    let days = nested(&body, "entropy")
        .get("auto_postpone_period_in_days")
        .and_then(Value::as_i64);
    let days = match days {
        Some(days) => days,
        None if form => return Ok(StatusCode::UNPROCESSABLE_ENTITY.into_response()),
        None => return Err(bad("auto_postpone_period_in_days is required")),
    };
    let days = match valid_entropy(days) {
        Ok(days) => days,
        Err(_) if form => return Ok(StatusCode::UNPROCESSABLE_ENTITY.into_response()),
        Err(error) => return Err(error),
    };
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("UPDATE accounts SET auto_postpone_period_in_days=? WHERE id=?")
        .bind(days)
        .bind(&p.account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("UPDATE boards SET auto_postpone_period_in_days=?,updated_at=? WHERE account_id=? AND entropy_override=0")
        .bind(days).bind(now()).bind(&p.account_id).execute(&mut *tx).await.map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(if form {
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/account/settings"))],
        )
            .into_response()
    } else {
        Json(account_json(&app, &p).await?).into_response()
    })
}

#[derive(Deserialize, Default)]
struct PageQuery {
    page: Option<i64>,
}
async fn list_boards(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let scope =
        " FROM boards b JOIN accesses x ON x.board_id=b.id WHERE b.account_id=? AND x.user_id=?";
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*){scope}"))
        .bind(&p.account_id)
        .bind(&p.user_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let rows = sqlx::query(&format!(
        "SELECT b.*{scope} ORDER BY x.accessed_at DESC,b.created_at DESC LIMIT ? OFFSET ?"
    ))
    .bind(&p.account_id)
    .bind(&p.user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let host = base(&headers);
    let mut out = Vec::new();
    for r in &rows {
        out.push(board_json(&app, &p, r, &host).await?);
    }
    Ok(paginated(json!(out), total, page, &uri, &headers))
}
async fn show_board(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let r = board(&app, &p, &id).await?;
    if !headers.contains_key("x-rizzy-json-request") {
        let mut query = parse_card_query(raw.clone());
        if let Some(filter_id) = query.filter_id.as_deref() {
            let fields: String = sqlx::query_scalar(
                "SELECT fields FROM filters WHERE id=? AND account_id=? AND creator_id=?",
            )
            .bind(filter_id)
            .bind(&p.account_id)
            .bind(&p.user_id)
            .fetch_optional(&app.db)
            .await
            .map_err(db_err)?
            .ok_or_else(missing)?;
            let stored: Value = serde_json::from_str(&fields)
                .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Invalid saved filter"))?;
            query = card_query_from_filter(&stored, query.page);
        }
        query.terms.retain(|term| !term.trim().is_empty());
        let filtered = board_filter_used(&query);
        query.board_ids = vec![id.clone()];
        return Ok(if filtered {
            board_page::show_filtered(&app, &p, &r, &query, raw.as_deref())
                .await?
                .into_response()
        } else {
            board_page::show(&app, &p, &r).await?.into_response()
        });
    }
    Ok(Json(board_detail(&app, &p, &r, &base(&headers)).await?).into_response())
}
fn board_filter_used(query: &CardQuery) -> bool {
    !query.terms.is_empty()
        || !query.card_ids.is_empty()
        || !query.tag_ids.is_empty()
        || !query.assignee_ids.is_empty()
        || !query.creator_ids.is_empty()
        || !query.closer_ids.is_empty()
        || query.assignment_status.as_deref() == Some("unassigned")
        || query
            .indexed_by
            .as_deref()
            .is_some_and(|index| index != "all")
        || query
            .sorted_by
            .as_deref()
            .is_some_and(|sort| sort != "latest")
}
async fn list_board_accesses(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let b = board(&app, &p, &id).await?;
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM users WHERE account_id=? AND active=1 AND role!='system'",
    )
    .bind(&p.account_id)
    .fetch_one(&app.db)
    .await
    .map_err(db_err)?;
    let rows=sqlx::query("SELECT u.*,i.email_address,x.involvement FROM users u JOIN identities i ON i.id=u.identity_id LEFT JOIN accesses x ON x.user_id=u.id AND x.board_id=? WHERE u.account_id=? AND u.active=1 AND u.role!='system' ORDER BY lower(u.name),u.id LIMIT ? OFFSET ?")
        .bind(&id).bind(&p.account_id).bind(limit).bind(offset).fetch_all(&app.db).await.map_err(db_err)?;
    let host = base(&headers);
    let users: Vec<Value> = rows
        .iter()
        .map(|r| {
            let mut value = user_json_row(&p, r, &host);
            let involvement: Option<String> = r.get("involvement");
            value["has_access"] = json!(involvement.is_some());
            value["involvement"] = json!(involvement);
            value
        })
        .collect();
    Ok(paginated(
        json!({"board_id":id,"all_access":b.get::<i64,_>("all_access")!=0,"users":users}),
        total,
        page,
        &uri,
        &headers,
    ))
}
async fn update_board_involvement(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    board(&app, &p, &id).await?;
    let involvement = required(&body, "involvement")?;
    if !["access_only", "watching"].contains(&involvement.as_str()) {
        return Err(bad("Invalid involvement"));
    }
    let changed = sqlx::query("UPDATE accesses SET involvement=? WHERE board_id=? AND user_id=?")
        .bind(&involvement)
        .bind(&id)
        .bind(&p.user_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    if changed.rows_affected() == 0 {
        return Err(missing());
    }
    Ok(StatusCode::NO_CONTENT)
}
const WEBHOOK_ACTIONS: &[&str] = &[
    "card_assigned",
    "card_closed",
    "card_postponed",
    "card_auto_postponed",
    "card_board_changed",
    "card_published",
    "card_reopened",
    "card_sent_back_to_triage",
    "card_triaged",
    "card_unassigned",
    "comment_created",
];
fn normalized_webhook_actions(value: &Value) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(actions) = value.as_array() {
        for action in actions.iter().filter_map(Value::as_str) {
            if WEBHOOK_ACTIONS.contains(&action) && !out.iter().any(|existing| existing == action) {
                out.push(action.to_owned());
            }
        }
    }
    out
}
fn webhook_secret() -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut rng = rand::rng();
    (0..24)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}
fn validate_webhook_url(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    let parsed = url::Url::parse(trimmed).map_err(|_| bad("url is not a URL"))?;
    if !["http", "https"].contains(&parsed.scheme()) || parsed.host_str().is_none() {
        return Err(bad("url must use HTTP or HTTPS"));
    }
    Ok(trimmed.to_owned())
}
async fn webhook_json(
    app: &App,
    p: &Principal,
    b: &SqliteRow,
    w: &SqliteRow,
    host: &str,
) -> Result<Value> {
    let id: String = w.get("id");
    let board_id: String = b.get("id");
    let actions: Value =
        serde_json::from_str(&w.get::<String, _>("subscribed_actions")).unwrap_or(json!([]));
    Ok(
        json!({"id":id,"name":w.get::<String,_>("name"),"active":w.get::<i64,_>("active")!=0,
        "signing_secret":w.get::<String,_>("signing_secret"),"subscribed_actions":actions,
        "payload_url":w.get::<String,_>("url"),"created_at":w.get::<String,_>("created_at"),
        "url":format!("{host}/{}/boards/{board_id}/webhooks/{id}",p.slug),
        "board":board_json(app,p,b,host).await?}),
    )
}
async fn webhook_row(
    app: &App,
    p: &Principal,
    board_id: &str,
    webhook_id: &str,
) -> Result<SqliteRow> {
    sqlx::query("SELECT * FROM webhooks WHERE id=? AND board_id=? AND account_id=?")
        .bind(webhook_id)
        .bind(board_id)
        .bind(&p.account_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)
}
async fn list_webhooks(
    State(app): Shared,
    Path((account, board_id)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let b = board(&app, &p, &board_id).await?;
    p.account_admin()?;
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM webhooks WHERE board_id=? AND account_id=?")
            .bind(&board_id)
            .bind(&p.account_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
    let rows=sqlx::query("SELECT * FROM webhooks WHERE board_id=? AND account_id=? ORDER BY name ASC,id DESC LIMIT ? OFFSET ?")
        .bind(&board_id).bind(&p.account_id).bind(limit).bind(offset).fetch_all(&app.db).await.map_err(db_err)?;
    if !headers.contains_key("x-rizzy-json-request") {
        return Ok(
            webhook_page::index(p.slug, &board_id, &rows, page, total, offset, limit)
                .into_response(),
        );
    }
    let host = base(&headers);
    let mut out = Vec::new();
    for row in &rows {
        out.push(webhook_json(&app, &p, &b, row, &host).await?);
    }
    Ok(paginated(json!(out), total, page, &uri, &headers))
}
async fn show_webhook(
    State(app): Shared,
    Path((account, board_id, webhook_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let b = board(&app, &p, &board_id).await?;
    p.account_admin()?;
    let w = webhook_row(&app, &p, &board_id, &webhook_id).await?;
    if !headers.contains_key("x-rizzy-json-request") {
        let deliveries = sqlx::query("SELECT state,created_at FROM webhook_deliveries WHERE webhook_id=? ORDER BY created_at DESC,id DESC LIMIT 20")
            .bind(&webhook_id).fetch_all(&app.db).await.map_err(db_err)?;
        return Ok(webhook_page::show(p.slug, &board_id, &w, &deliveries).into_response());
    }
    Ok(Json(webhook_json(&app, &p, &b, &w, &base(&headers)).await?).into_response())
}
fn webhook_body(headers: &HeaderMap, raw: &Bytes) -> Result<(Value, bool)> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.starts_with("application/x-www-form-urlencoded"));
    if !form {
        return serde_json::from_slice(raw)
            .map(|body| (body, false))
            .map_err(|_| bad("Invalid JSON"));
    }
    let mut webhook = serde_json::Map::new();
    let mut actions = Vec::new();
    let mut method = None;
    for (key, value) in url::form_urlencoded::parse(raw) {
        match key.as_ref() {
            "webhook[name]" => {
                webhook.insert("name".into(), json!(value.into_owned()));
            }
            "webhook[url]" => {
                webhook.insert("url".into(), json!(value.into_owned()));
            }
            "webhook[subscribed_actions][]" => actions.push(value.into_owned()),
            "_method" => method = Some(value.into_owned()),
            _ => {}
        }
    }
    webhook.insert("subscribed_actions".into(), json!(actions));
    let mut body = json!({"webhook": webhook});
    if let Some(method) = method {
        body["_method"] = json!(method);
    }
    Ok((body, true))
}
async fn create_webhook(
    State(app): Shared,
    Path((account, board_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let (body, form) = webhook_body(&headers, &body)?;
    let p = principal(&app, &headers, &account).await?;
    let b = board(&app, &p, &board_id).await?;
    p.write()?;
    p.account_admin()?;
    let v = nested(&body, "webhook");
    let name = string(v, "name").unwrap_or_default();
    let url = string(v, "url").unwrap_or_default();
    let actions = normalized_webhook_actions(v.get("subscribed_actions").unwrap_or(&Value::Null));
    if name.trim().is_empty() || validate_webhook_url(&url).is_err() {
        if form {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                webhook_page::invalid_form(
                    p.slug,
                    &board_id,
                    None,
                    "Name and a valid http:// or https:// URL are required",
                    &name,
                    &url,
                    &actions,
                ),
            )
                .into_response());
        }
        return Err(bad("Name and a valid URL are required"));
    }
    let url = validate_webhook_url(&url)?;
    let webhook_id = id();
    let timestamp = now();
    sqlx::query("INSERT INTO webhooks(id,account_id,board_id,name,url,signing_secret,subscribed_actions,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind(&webhook_id).bind(&p.account_id).bind(&board_id).bind(name).bind(url).bind(webhook_secret())
        .bind(json!(actions).to_string()).bind(&timestamp).bind(&timestamp).execute(&app.db).await.map_err(db_err)?;
    let w = webhook_row(&app, &p, &board_id, &webhook_id).await?;
    if form {
        return Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("/{account}/boards/{board_id}/webhooks/{webhook_id}"),
            )],
        )
            .into_response());
    }
    let host = base(&headers);
    Ok(created(
        format!("{host}/{account}/boards/{board_id}/webhooks/{webhook_id}.json"),
        webhook_json(&app, &p, &b, &w, &host).await?,
    ))
}
async fn update_webhook(
    State(app): Shared,
    Path((account, board_id, webhook_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Response> {
    let (body, form) = webhook_body(&headers, &body)?;
    if method == Method::POST {
        if !form {
            return Err(missing());
        }
        match body.get("_method").and_then(Value::as_str) {
            Some("delete") => {
                return delete_webhook(State(app), Path((account, board_id, webhook_id)), headers)
                    .await;
            }
            Some("put" | "patch") => {}
            _ => return Err(missing()),
        }
    }
    let p = principal(&app, &headers, &account).await?;
    let b = board(&app, &p, &board_id).await?;
    p.write()?;
    p.account_admin()?;
    let w = webhook_row(&app, &p, &board_id, &webhook_id).await?;
    let v = nested(&body, "webhook");
    let name = string(v, "name").unwrap_or_else(|| w.get("name"));
    if name.trim().is_empty() {
        if form {
            let selected =
                normalized_webhook_actions(v.get("subscribed_actions").unwrap_or(&Value::Null));
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                webhook_page::invalid_form(
                    p.slug,
                    &board_id,
                    Some(&w),
                    "Name can't be blank",
                    &name,
                    "",
                    &selected,
                ),
            )
                .into_response());
        }
        return Err(bad("name is required"));
    }
    let actions = v
        .get("subscribed_actions")
        .map(normalized_webhook_actions)
        .unwrap_or_else(|| {
            serde_json::from_str(&w.get::<String, _>("subscribed_actions")).unwrap_or_default()
        });
    sqlx::query("UPDATE webhooks SET name=?,subscribed_actions=?,updated_at=? WHERE id=?")
        .bind(name)
        .bind(json!(actions).to_string())
        .bind(now())
        .bind(&webhook_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    let updated = webhook_row(&app, &p, &board_id, &webhook_id).await?;
    if form {
        return Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("/{account}/boards/{board_id}/webhooks/{webhook_id}"),
            )],
        )
            .into_response());
    }
    Ok(Json(webhook_json(&app, &p, &b, &updated, &base(&headers)).await?).into_response())
}
async fn delete_webhook(
    State(app): Shared,
    Path((account, board_id, webhook_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    board(&app, &p, &board_id).await?;
    p.write()?;
    p.account_admin()?;
    let deleted = sqlx::query("DELETE FROM webhooks WHERE id=? AND board_id=? AND account_id=?")
        .bind(&webhook_id)
        .bind(&board_id)
        .bind(&p.account_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    if deleted.rows_affected() == 0 {
        return Err(missing());
    }
    if headers.contains_key("x-rizzy-json-request") {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("/{account}/boards/{board_id}/webhooks"),
            )],
        )
            .into_response())
    }
}
async fn activate_webhook(
    State(app): Shared,
    Path((account, board_id, webhook_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let b = board(&app, &p, &board_id).await?;
    p.write()?;
    p.account_admin()?;
    let changed = sqlx::query(
        "UPDATE webhooks SET active=1,updated_at=? WHERE id=? AND board_id=? AND account_id=?",
    )
    .bind(now())
    .bind(&webhook_id)
    .bind(&board_id)
    .bind(&p.account_id)
    .execute(&app.db)
    .await
    .map_err(db_err)?;
    if changed.rows_affected() == 0 {
        return Err(missing());
    }
    let w = webhook_row(&app, &p, &board_id, &webhook_id).await?;
    if !headers.contains_key("x-rizzy-json-request") {
        return Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("/{account}/boards/{board_id}/webhooks/{webhook_id}"),
            )],
        )
            .into_response());
    }
    Ok((
        StatusCode::CREATED,
        Json(webhook_json(&app, &p, &b, &w, &base(&headers)).await?),
    )
        .into_response())
}
async fn list_webhook_deliveries(
    State(app): Shared,
    Path((account, board_id, webhook_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    board(&app, &p, &board_id).await?;
    p.account_admin()?;
    webhook_row(&app, &p, &board_id, &webhook_id).await?;
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let total: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM webhook_deliveries WHERE webhook_id=? AND account_id=?",
    )
    .bind(&webhook_id)
    .bind(&p.account_id)
    .fetch_one(&app.db)
    .await
    .map_err(db_err)?;
    let rows=sqlx::query("SELECT d.*,e.action,e.created_at event_created_at,e.creator_id,e.eventable_type,e.eventable_id,c.number,u.name creator_name FROM webhook_deliveries d JOIN events e ON e.id=d.event_id JOIN cards c ON c.id=e.card_id JOIN users u ON u.id=e.creator_id WHERE d.webhook_id=? AND d.account_id=? ORDER BY d.created_at DESC,d.id DESC LIMIT ? OFFSET ?")
        .bind(&webhook_id).bind(&p.account_id).bind(limit).bind(offset).fetch_all(&app.db).await.map_err(db_err)?;
    let host = base(&headers);
    let mut out = Vec::new();
    for row in rows {
        let eventable_type: String = row.get("eventable_type");
        let eventable_id: String = row.get("eventable_id");
        let eventable_url = if eventable_type == "Comment" {
            format!(
                "{host}/{account}/cards/{}/comments/{eventable_id}",
                row.get::<i64, _>("number")
            )
        } else {
            format!("{host}/{account}/cards/{}", row.get::<i64, _>("number"))
        };
        let request: Option<Value> = row
            .get::<Option<String>, _>("request")
            .and_then(|s| serde_json::from_str(&s).ok());
        let response: Option<Value> = row
            .get::<Option<String>, _>("response")
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .map(|value| json!({"code":value.get("code"),"error":value.get("error")}));
        out.push(json!({"id":row.get::<String,_>("id"),"state":row.get::<String,_>("state"),
            "created_at":row.get::<String,_>("created_at"),"updated_at":row.get::<String,_>("updated_at"),
            "request":request,"response":response,"event":{"id":row.get::<String,_>("event_id"),
            "action":row.get::<String,_>("action"),"created_at":row.get::<String,_>("event_created_at"),
            "creator":{"id":row.get::<String,_>("creator_id"),"name":row.get::<String,_>("creator_name")},
            "eventable":{"type":eventable_type,"id":eventable_id,"url":eventable_url}}}));
    }
    Ok(paginated(json!(out), total, page, &uri, &headers))
}
fn board_body(headers: &HeaderMap, body: &Bytes) -> Result<(Value, bool)> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded");
    if !form {
        return Ok((
            serde_json::from_slice(body).map_err(|_| bad("Invalid JSON"))?,
            false,
        ));
    }
    let mut fields = serde_json::Map::new();
    let mut board = serde_json::Map::new();
    let mut users = Vec::new();
    for (key, value) in url::form_urlencoded::parse(body) {
        match key.as_ref() {
            "board[name]" | "board[public_description]" => {
                board.insert(
                    key.trim_start_matches("board[")
                        .trim_end_matches(']')
                        .to_owned(),
                    json!(value.into_owned()),
                );
            }
            "board[all_access]" => {
                board.insert(
                    "all_access".into(),
                    json!(matches!(value.as_ref(), "1" | "true" | "on")),
                );
            }
            "board[auto_postpone_period_in_days]" => {
                if let Ok(days) = value.parse::<i64>() {
                    board.insert("auto_postpone_period_in_days".into(), json!(days));
                }
            }
            "user_ids[]" => users.push(json!(value.into_owned())),
            "_method" => {
                fields.insert("_method".into(), json!(value.into_owned()));
            }
            _ => {}
        }
    }
    fields.insert("board".into(), Value::Object(board));
    if !users.is_empty() {
        fields.insert("user_ids".into(), Value::Array(users));
    }
    Ok((Value::Object(fields), true))
}
async fn create_board(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let (body, form) = board_body(&headers, &body)?;
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let v = nested(&body, "board");
    let name = required(v, "name")?;
    let all = v.get("all_access").and_then(Value::as_bool).unwrap_or(true);
    let requested_days = v
        .get("auto_postpone_period_in_days")
        .and_then(Value::as_i64);
    let account_days: i64 =
        sqlx::query_scalar("SELECT auto_postpone_period_in_days FROM accounts WHERE id=?")
            .bind(&p.account_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
    let days = valid_entropy(requested_days.unwrap_or(account_days))?;
    let id = id();
    let timestamp = now();
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let public_description = string(v, "public_description").unwrap_or_default();
    sqlx::query("INSERT INTO boards(id,account_id,creator_id,name,all_access,auto_postpone_period_in_days,entropy_override,public_description,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?)")
        .bind(&id).bind(&p.account_id).bind(&p.user_id).bind(name).bind(all as i64).bind(days).bind(requested_days.is_some() as i64).bind(public_description).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
    sqlx::query("INSERT INTO accesses(board_id,user_id,involvement) VALUES(?,?,'watching')")
        .bind(&id)
        .bind(&p.user_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    if all {
        sqlx::query("INSERT OR IGNORE INTO accesses(board_id,user_id) SELECT ?,id FROM users WHERE account_id=? AND active=1 AND role!='system'")
            .bind(&id).bind(&p.account_id).execute(&mut *tx).await.map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    let r = board(&app, &p, &id).await?;
    if form {
        return Ok((
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/boards/{id}"))],
        )
            .into_response());
    }
    Ok(created(
        format!("/{account}/boards/{id}.json"),
        board_detail(&app, &p, &r, &base(&headers)).await?,
    ))
}
async fn update_board(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    method: Method,
    body: Bytes,
) -> Result<Response> {
    let (body, form) = board_body(&headers, &body)?;
    if method == Method::POST
        && (!form || body.get("_method").and_then(Value::as_str) != Some("patch"))
    {
        return Err(missing());
    }
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let b = board(&app, &p, &id).await?;
    p.admin(&b.get::<String, _>("creator_id"))?;
    let v = nested(&body, "board");
    let name = string(v, "name").unwrap_or_else(|| b.get("name"));
    if name.trim().is_empty() {
        return Err(bad("name is required"));
    }
    let all = v
        .get("all_access")
        .and_then(Value::as_bool)
        .unwrap_or(b.get::<i64, _>("all_access") != 0);
    let requested_days = v
        .get("auto_postpone_period_in_days")
        .and_then(Value::as_i64);
    let days = valid_entropy(requested_days.unwrap_or(b.get("auto_postpone_period_in_days")))?;
    let override_days = requested_days.is_some() || b.get::<i64, _>("entropy_override") != 0;
    let public_description =
        string(v, "public_description").unwrap_or_else(|| b.get("public_description"));
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("UPDATE boards SET name=?,all_access=?,auto_postpone_period_in_days=?,entropy_override=?,public_description=?,updated_at=? WHERE id=?")
        .bind(name).bind(all as i64).bind(days).bind(override_days as i64).bind(public_description).bind(now()).bind(&id).execute(&mut *tx).await.map_err(db_err)?;
    if all && b.get::<i64, _>("all_access") == 0 {
        sqlx::query("INSERT OR IGNORE INTO accesses(board_id,user_id) SELECT ?,id FROM users WHERE account_id=? AND active=1 AND role!='system'")
            .bind(&id).bind(&p.account_id).execute(&mut *tx).await.map_err(db_err)?;
    }
    if let Some(ids) = body
        .get("user_ids")
        .or_else(|| v.get("user_ids"))
        .and_then(Value::as_array)
    {
        let wanted: HashSet<&str> = ids.iter().filter_map(Value::as_str).collect();
        for user in ids.iter().filter_map(Value::as_str) {
            sqlx::query("INSERT OR IGNORE INTO accesses(board_id,user_id) SELECT ?,id FROM users WHERE id=? AND account_id=? AND active=1 AND role!='system'")
                .bind(&id).bind(user).bind(&p.account_id).execute(&mut *tx).await.map_err(db_err)?;
        }
        if !all {
            let current_users: Vec<String> =
                sqlx::query_scalar("SELECT user_id FROM accesses WHERE board_id=?")
                    .bind(&id)
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(db_err)?;
            for user in current_users
                .iter()
                .filter(|user| !wanted.contains(user.as_str()))
            {
                sqlx::query("DELETE FROM accesses WHERE board_id=? AND user_id=?")
                    .bind(&id)
                    .bind(user)
                    .execute(&mut *tx)
                    .await
                    .map_err(db_err)?;
                sqlx::query("DELETE FROM watches WHERE user_id=? AND card_id IN (SELECT id FROM cards WHERE board_id=?)")
                    .bind(user).bind(&id).execute(&mut *tx).await.map_err(db_err)?;
                sqlx::query("DELETE FROM mentions WHERE mentionee_id=? AND ((source_type='Card' AND source_id IN (SELECT id FROM cards WHERE board_id=?)) OR (source_type='Comment' AND source_id IN (SELECT m.id FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.board_id=?)))")
                    .bind(user).bind(&id).bind(&id).execute(&mut *tx).await.map_err(db_err)?;
                sqlx::query("DELETE FROM notifications WHERE user_id=? AND card_id IN (SELECT id FROM cards WHERE board_id=?)")
                    .bind(user).bind(&id).execute(&mut *tx).await.map_err(db_err)?;
                sqlx::query("DELETE FROM pins WHERE user_id=? AND card_id IN (SELECT id FROM cards WHERE board_id=?)")
                    .bind(user).bind(&id).execute(&mut *tx).await.map_err(db_err)?;
                sqlx::query("DELETE FROM assignments WHERE assignee_id=? AND card_id IN (SELECT id FROM cards WHERE board_id=?)")
                    .bind(user).bind(&id).execute(&mut *tx).await.map_err(db_err)?;
            }
        }
    }
    tx.commit().await.map_err(db_err)?;
    if !all {
        let has_access: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM accesses WHERE board_id=? AND user_id=?")
                .bind(&id)
                .bind(&p.user_id)
                .fetch_one(&app.db)
                .await
                .map_err(db_err)?;
        if has_access == 0 {
            return Ok(if form {
                (
                    StatusCode::FOUND,
                    [(header::LOCATION, format!("/{account}/"))],
                )
                    .into_response()
            } else {
                StatusCode::NO_CONTENT.into_response()
            });
        }
    }
    let r = board(&app, &p, &id).await?;
    if form {
        return Ok((
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/boards/{id}/edit"))],
        )
            .into_response());
    }
    Ok(Json(board_detail(&app, &p, &r, &base(&headers)).await?).into_response())
}
async fn update_board_entropy(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let b = board(&app, &p, &id).await?;
    p.admin(&b.get::<String, _>("creator_id"))?;
    let days = nested(&body, "board")
        .get("auto_postpone_period_in_days")
        .and_then(Value::as_i64)
        .ok_or_else(|| bad("auto_postpone_period_in_days is required"))?;
    let days = valid_entropy(days)?;
    sqlx::query("UPDATE boards SET auto_postpone_period_in_days=?,entropy_override=1,updated_at=? WHERE id=? AND account_id=?")
        .bind(days).bind(now()).bind(&id).bind(&p.account_id).execute(&app.db).await.map_err(db_err)?;
    let b = board(&app, &p, &id).await?;
    Ok(Json(board_detail(&app, &p, &b, &base(&headers)).await?))
}
async fn delete_board(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let b = board(&app, &p, &id).await?;
    p.admin(&b.get::<String, _>("creator_id"))?;
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let keys = storage::purge_card_storage_tx(&mut tx, &p.account_id, None, Some(&id)).await?;
    sqlx::query("DELETE FROM boards WHERE id=? AND account_id=?")
        .bind(id)
        .bind(&p.account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    storage::remove_files(keys).await;
    Ok(if headers.contains_key("x-rizzy-json-request") {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/"))],
        )
            .into_response()
    })
}
fn publication_key() -> String {
    const ALPHABET: &[u8] = b"123456789ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz";
    let mut rng = rand::rng();
    (0..24)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}
async fn publish_board(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let b = board(&app, &p, &id).await?;
    p.admin(&b.get::<String, _>("creator_id"))?;
    if b.get::<Option<String>, _>("publication_key").is_none() {
        sqlx::query("UPDATE boards SET publication_key=?,updated_at=? WHERE id=? AND account_id=?")
            .bind(publication_key())
            .bind(now())
            .bind(&id)
            .bind(&p.account_id)
            .execute(&app.db)
            .await
            .map_err(db_err)?;
    }
    let b = board(&app, &p, &id).await?;
    Ok((
        StatusCode::CREATED,
        Json(board_json(&app, &p, &b, &base(&headers)).await?),
    )
        .into_response())
}
async fn unpublish_board(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let b = board(&app, &p, &id).await?;
    p.admin(&b.get::<String, _>("creator_id"))?;
    sqlx::query("UPDATE boards SET publication_key=NULL,updated_at=? WHERE id=? AND account_id=?")
        .bind(now())
        .bind(&id)
        .bind(&p.account_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn published_board(app: &App, account: &str, key: &str) -> Result<SqliteRow> {
    let account: i64 = account.parse().map_err(|_| missing())?;
    sqlx::query("SELECT b.* FROM boards b JOIN accounts a ON a.id=b.account_id WHERE a.external_account_id=? AND b.publication_key=?")
        .bind(account).bind(key).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)
}
fn public_page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>body{{max-width:48rem;margin:3rem auto;padding:0 1.5rem;font:1.05rem/1.55 system-ui,sans-serif;color:#21211f;background:#faf9f6}}h1{{line-height:1.15}}a{{color:#244789}}li{{padding:.7rem 0;border-bottom:1px solid #ddd}}nav{{margin-bottom:2rem}}article{{white-space:pre-wrap}}</style></head><body>{body}</body></html>",
        escape(title)
    ))
}
async fn public_board(
    State(app): Shared,
    Path((account, key)): Path<(String, String)>,
) -> Result<Html<String>> {
    let board = published_board(&app, &account, &key).await?;
    let board_id: String = board.get("id");
    let rows = sqlx::query("SELECT number,title FROM cards WHERE board_id=? AND status='published' AND closed=0 AND postponed=0 ORDER BY golden DESC,last_active_at DESC LIMIT 100")
        .bind(board_id).fetch_all(&app.db).await.map_err(db_err)?;
    let columns = sqlx::query("SELECT id,name FROM columns WHERE board_id=? ORDER BY position,id")
        .bind(board.get::<String, _>("id"))
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let mut body = format!(
        "<h1>{}</h1><p>{}</p><nav><a href=\"/{account}/public/boards/{key}/columns/stream\">Maybe?</a> · <a href=\"/{account}/public/boards/{key}/columns/not_now\">Not now</a> · <a href=\"/{account}/public/boards/{key}/columns/closed\">Done</a>",
        escape(&board.get::<String, _>("name")),
        escape(&board.get::<String, _>("public_description"))
    );
    for column in columns {
        body.push_str(&format!(
            " · <a href=\"/{account}/public/boards/{key}/columns/{}\">{}</a>",
            column.get::<String, _>("id"),
            escape(&column.get::<String, _>("name"))
        ));
    }
    body.push_str("</nav><ul>");
    for row in rows {
        let number: i64 = row.get("number");
        body.push_str(&format!(
            "<li><a href=\"/{account}/public/boards/{key}/cards/{number}\">{}</a></li>",
            escape(&row.get::<String, _>("title"))
        ));
    }
    body.push_str("</ul>");
    Ok(public_page(&board.get::<String, _>("name"), &body))
}
async fn public_card(
    State(app): Shared,
    Path((account, key, number)): Path<(String, String, String)>,
) -> Result<Html<String>> {
    let board = published_board(&app, &account, &key).await?;
    let number: i64 = number.parse().map_err(|_| missing())?;
    let card = sqlx::query(
        "SELECT title,description FROM cards WHERE board_id=? AND number=? AND status='published'",
    )
    .bind(board.get::<String, _>("id"))
    .bind(number)
    .fetch_optional(&app.db)
    .await
    .map_err(db_err)?
    .ok_or_else(missing)?;
    let title: String = card.get("title");
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let (_, description_html, _) = storage::render_rich_text(
        &app,
        &board.get::<String, _>("account_id"),
        slug,
        &card.get::<String, _>("description"),
    )
    .await?;
    let body = format!(
        "<nav><a href=\"/{account}/public/boards/{key}\">{}</a></nav><h1>{}</h1><article>{}</article>",
        escape(&board.get::<String, _>("name")),
        escape(&title),
        description_html
    );
    Ok(public_page(&title, &body))
}

async fn checked_column(
    app: &App,
    p: &Principal,
    board_id: &str,
    column_id: &str,
) -> Result<SqliteRow> {
    board(app, p, board_id).await?;
    sqlx::query("SELECT * FROM columns WHERE id=? AND board_id=? AND account_id=?")
        .bind(column_id.trim_end_matches(".json"))
        .bind(board_id)
        .bind(&p.account_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)
}
async fn list_columns(
    State(app): Shared,
    Path((account, board_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    board(&app, &p, &board_id).await?;
    let rows = sqlx::query("SELECT * FROM columns WHERE board_id=? ORDER BY position,id")
        .bind(board_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    Ok(Json(json!(
        rows.iter()
            .map(|r| column_json(&p, r, &base(&headers)))
            .collect::<Vec<_>>()
    )))
}
async fn show_column(
    State(app): Shared,
    Path((account, board_id, column_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    let c = checked_column(&app, &p, &board_id, &column_id).await?;
    Ok(Json(column_json(&p, &c, &base(&headers))))
}
async fn create_column(
    State(app): Shared,
    Path((account, board_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    board(&app, &p, &board_id).await?;
    let v = nested(&body, "column");
    let name = required(v, "name")?;
    let color = string(v, "color").unwrap_or_else(|| "var(--color-card-default)".into());
    let position: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(position),-1)+1 FROM columns WHERE board_id=?")
            .bind(&board_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
    let id = id();
    let timestamp = now();
    sqlx::query("INSERT INTO columns(id,account_id,board_id,name,color,position,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
        .bind(&id).bind(&p.account_id).bind(&board_id).bind(name).bind(color).bind(position).bind(&timestamp).bind(&timestamp).execute(&app.db).await.map_err(db_err)?;
    let c = checked_column(&app, &p, &board_id, &id).await?;
    Ok(created(
        format!("/{account}/boards/{board_id}/columns/{id}.json"),
        column_json(&p, &c, &base(&headers)),
    ))
}
async fn update_column(
    State(app): Shared,
    Path((account, board_id, column_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = checked_column(&app, &p, &board_id, &column_id).await?;
    let v = nested(&body, "column");
    let name = string(v, "name").unwrap_or_else(|| c.get("name"));
    if name.trim().is_empty() {
        return Err(bad("name is required"));
    }
    let color = string(v, "color").unwrap_or_else(|| c.get("color"));
    sqlx::query("UPDATE columns SET name=?,color=?,updated_at=? WHERE id=? AND board_id=?")
        .bind(name)
        .bind(color)
        .bind(now())
        .bind(&column_id)
        .bind(&board_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    let c = checked_column(&app, &p, &board_id, &column_id).await?;
    Ok(Json(column_json(&p, &c, &base(&headers))))
}
async fn delete_column(
    State(app): Shared,
    Path((account, board_id, column_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    checked_column(&app, &p, &board_id, &column_id).await?;
    sqlx::query("DELETE FROM columns WHERE id=? AND board_id=?")
        .bind(column_id)
        .bind(board_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn move_column(
    app: &App,
    account: &str,
    column_id: &str,
    headers: &HeaderMap,
    left: bool,
) -> Result<StatusCode> {
    let p = principal(app, headers, account).await?;
    p.write()?;
    let column = sqlx::query("SELECT * FROM columns WHERE id=? AND account_id=?")
        .bind(column_id)
        .bind(&p.account_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)?;
    let board_id: String = column.get("board_id");
    board(app, &p, &board_id).await?;

    let _guard = app.write_lock.lock().await;
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let position: i64 = sqlx::query_scalar("SELECT position FROM columns WHERE id=?")
        .bind(column_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
    let neighbor = if left {
        sqlx::query("SELECT id,position FROM columns WHERE board_id=? AND position<? ORDER BY position DESC LIMIT 1")
            .bind(&board_id).bind(position).fetch_optional(&mut *tx).await.map_err(db_err)?
    } else {
        sqlx::query("SELECT id,position FROM columns WHERE board_id=? AND position>? ORDER BY position ASC LIMIT 1")
            .bind(&board_id).bind(position).fetch_optional(&mut *tx).await.map_err(db_err)?
    };
    if let Some(neighbor) = neighbor {
        let neighbor_id: String = neighbor.get("id");
        let neighbor_position: i64 = neighbor.get("position");
        sqlx::query("UPDATE columns SET position=? WHERE id=?")
            .bind(neighbor_position)
            .bind(column_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query("UPDATE columns SET position=? WHERE id=?")
            .bind(position)
            .bind(neighbor_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(StatusCode::CREATED)
}

async fn move_column_left(
    State(app): Shared,
    Path((account, column_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    move_column(&app, &account, &column_id, &headers, true).await
}

async fn move_column_right(
    State(app): Shared,
    Path((account, column_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    move_column(&app, &account, &column_id, &headers, false).await
}

#[derive(Default)]
struct CardQuery {
    filter_id: Option<String>,
    indexed_by: Option<String>,
    sorted_by: Option<String>,
    creation: Option<String>,
    closure: Option<String>,
    board_ids: Vec<String>,
    column_ids: Vec<String>,
    tag_ids: Vec<String>,
    assignee_ids: Vec<String>,
    creator_ids: Vec<String>,
    closer_ids: Vec<String>,
    card_ids: Vec<String>,
    terms: Vec<String>,
    assignment_status: Option<String>,
    page: i64,
}
fn parse_card_query(raw: Option<String>) -> CardQuery {
    let mut q = CardQuery {
        page: 1,
        ..Default::default()
    };
    if let Some(raw) = raw {
        for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
            match key.as_ref() {
                "page" => q.page = value.parse::<i64>().unwrap_or(1).max(1),
                _ if value.trim().is_empty() => {}
                "filter_id" => q.filter_id = Some(value.into_owned()),
                "indexed_by" => q.indexed_by = Some(value.into_owned()),
                "sorted_by" => q.sorted_by = Some(value.into_owned()),
                "creation" => q.creation = Some(value.into_owned()),
                "closure" => q.closure = Some(value.into_owned()),
                "board_ids[]" => q.board_ids.push(value.into_owned()),
                "column_ids[]" => q.column_ids.push(value.into_owned()),
                "tag_ids[]" => q.tag_ids.push(value.into_owned()),
                "assignee_ids[]" => q.assignee_ids.push(value.into_owned()),
                "creator_ids[]" => q.creator_ids.push(value.into_owned()),
                "closer_ids[]" => q.closer_ids.push(value.into_owned()),
                "card_ids[]" => q.card_ids.push(value.into_owned()),
                "terms[]" => q.terms.push(value.into_owned()),
                "assignment_status" => q.assignment_status = Some(value.into_owned()),
                _ => {}
            }
        }
    }
    q
}
fn card_query_from_filter(fields: &Value, page: i64) -> CardQuery {
    let scalar = |key: &str| fields.get(key).and_then(Value::as_str).map(str::to_owned);
    let array = |key: &str| {
        fields
            .get(key)
            .and_then(Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    CardQuery {
        indexed_by: scalar("indexed_by"),
        sorted_by: scalar("sorted_by"),
        creation: scalar("creation"),
        closure: scalar("closure"),
        assignment_status: scalar("assignment_status"),
        board_ids: array("board_ids"),
        column_ids: array("column_ids"),
        tag_ids: array("tag_ids"),
        assignee_ids: array("assignee_ids"),
        creator_ids: array("creator_ids"),
        closer_ids: array("closer_ids"),
        card_ids: array("card_ids"),
        terms: array("terms"),
        page,
        ..Default::default()
    }
}
fn time_window(input: &str) -> Option<(String, String)> {
    let key = input.to_ascii_lowercase().replace([' ', '_', '-'], "");
    let today = Utc::now().date_naive();
    let week = today - chrono::Duration::days(today.weekday().num_days_from_monday() as i64);
    let month = today.with_day(1)?;
    let year = today.with_month(1)?.with_day(1)?;
    let (start, end) = match key.as_str() {
        "today" => (today, today + chrono::Duration::days(1)),
        "yesterday" => (today - chrono::Duration::days(1), today),
        "thisweek" => (week, week + chrono::Duration::days(7)),
        "lastweek" => (week - chrono::Duration::days(7), week),
        "thismonth" => (month, month.checked_add_months(chrono::Months::new(1))?),
        "lastmonth" => (month.checked_sub_months(chrono::Months::new(1))?, month),
        "thisyear" => (year, year.with_year(year.year() + 1)?),
        "lastyear" => (year.with_year(year.year() - 1)?, year),
        _ => return None,
    };
    let format = |date: chrono::NaiveDate| {
        date.and_hms_opt(0, 0, 0)
            .unwrap()
            .and_utc()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
    };
    Some((format(start), format(end)))
}
fn cards_query<'a>(select: &str, p: &'a Principal, q: &'a CardQuery) -> QueryBuilder<'a, Sqlite> {
    let mut builder = QueryBuilder::new(format!(
        "{select} FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.account_id="
    ));
    builder.push_bind(&p.account_id).push(" AND c.status='published' AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=").push_bind(&p.user_id).push(")");
    if q.indexed_by.as_deref() != Some("closed")
        && q.closure.as_deref().and_then(time_window).is_none()
        && q.closer_ids.is_empty()
        && q.card_ids.is_empty()
    {
        builder.push(" AND c.closed=0");
    }
    if q.indexed_by.as_deref() != Some("not_now") && q.card_ids.is_empty() {
        builder.push(" AND c.postponed=0");
    }
    match q.indexed_by.as_deref() {
        Some("closed") => {
            builder.push(" AND c.closed=1");
        }
        Some("not_now") => {
            builder.push(" AND c.postponed=1");
        }
        Some("maybe") => {
            builder.push(" AND c.column_id IS NULL AND c.closed=0 AND c.postponed=0");
        }
        Some("golden") => {
            builder.push(" AND c.golden=1");
        }
        Some("postponing_soon") => {
            builder.push(" AND julianday(c.last_active_at)>julianday('now')-b.auto_postpone_period_in_days AND julianday(c.last_active_at)<=julianday('now')-b.auto_postpone_period_in_days*0.75");
        }
        Some("stalled") => {
            let cutoff = (Utc::now() - chrono::Duration::days(14))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            builder.push(" AND c.closed=0 AND c.postponed=0 AND c.updated_at<=")
                .push_bind(cutoff.clone())
                .push(" AND EXISTS(SELECT 1 FROM card_activity_spikes s WHERE s.card_id=c.id AND s.updated_at<=")
                .push_bind(cutoff).push(")");
        }
        Some("draft") => {
            builder.push(" AND 1=0");
        }
        _ => {}
    }
    if !q.board_ids.is_empty() {
        builder.push(" AND c.board_id IN (");
        let mut separated = builder.separated(",");
        for id in &q.board_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
    if !q.column_ids.is_empty() {
        builder.push(" AND c.column_id IN (");
        let mut separated = builder.separated(",");
        for id in &q.column_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
    }
    for (column, values) in [("c.id", &q.card_ids), ("c.creator_id", &q.creator_ids)] {
        if !values.is_empty() {
            builder.push(" AND ").push(column).push(" IN (");
            let mut separated = builder.separated(",");
            for value in values {
                separated.push_bind(value);
            }
            separated.push_unseparated(")");
        }
    }
    if !q.tag_ids.is_empty() {
        builder
            .push(" AND EXISTS (SELECT 1 FROM taggings g WHERE g.card_id=c.id AND g.tag_id IN (");
        let mut separated = builder.separated(",");
        for value in &q.tag_ids {
            separated.push_bind(value);
        }
        separated.push_unseparated("))");
    }
    if !q.assignee_ids.is_empty() {
        builder.push(
            " AND EXISTS (SELECT 1 FROM assignments a WHERE a.card_id=c.id AND a.assignee_id IN (",
        );
        let mut separated = builder.separated(",");
        for value in &q.assignee_ids {
            separated.push_bind(value);
        }
        separated.push_unseparated("))");
    }
    if q.assignment_status.as_deref() == Some("unassigned") {
        builder.push(" AND NOT EXISTS (SELECT 1 FROM assignments a WHERE a.card_id=c.id)");
    }
    if let Some((start, end)) = q.creation.as_deref().and_then(time_window) {
        builder
            .push(" AND c.created_at>=")
            .push_bind(start)
            .push(" AND c.created_at<")
            .push_bind(end);
    }
    if let Some((start, end)) = q.closure.as_deref().and_then(time_window) {
        builder
            .push(" AND EXISTS (SELECT 1 FROM closures z WHERE z.card_id=c.id AND z.created_at>=")
            .push_bind(start)
            .push(" AND z.created_at<")
            .push_bind(end)
            .push(")");
    }
    if !q.closer_ids.is_empty() {
        builder
            .push(" AND EXISTS (SELECT 1 FROM closures z WHERE z.card_id=c.id AND z.user_id IN (");
        let mut separated = builder.separated(",");
        for id in &q.closer_ids {
            separated.push_bind(id);
        }
        separated.push_unseparated("))");
    }
    if !q.terms.is_empty() {
        let searches: Option<Vec<String>> = q.terms.iter().map(|term| fts_query(term)).collect();
        if let Some(searches) = searches {
            let search = searches.join(" AND ");
            builder.push(" AND (c.rowid IN (SELECT rowid FROM card_search WHERE card_search MATCH ")
                .push_bind(search.clone())
                .push(") OR c.id IN (SELECT m.card_id FROM comments m WHERE m.rowid IN (SELECT rowid FROM comment_search WHERE comment_search MATCH ")
                .push_bind(search).push(")))");
        } else {
            builder.push(" AND 1=0");
        }
    }
    builder
}
async fn list_tags(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM tags WHERE account_id=?")
        .bind(&p.account_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let rows = sqlx::query("SELECT id,title,created_at FROM tags WHERE account_id=? ORDER BY lower(title),id LIMIT ? OFFSET ?")
        .bind(&p.account_id).bind(limit).bind(offset).fetch_all(&app.db).await.map_err(db_err)?;
    let host = base(&headers);
    let items: Vec<Value> = rows.iter().map(|r| {
        let id: String = r.get("id");
        json!({"id":id,"title":r.get::<String,_>("title"),"created_at":r.get::<String,_>("created_at"),"url":format!("{host}/{}/cards?tag_ids%5B%5D={id}",p.slug)})
    }).collect();
    Ok(paginated(json!(items), total, page, &uri, &headers))
}
#[derive(Deserialize)]
struct SearchQuery {
    q: Option<String>,
    page: Option<i64>,
}
fn fts_query(raw: &str) -> Option<String> {
    let balanced = raw.chars().filter(|ch| *ch == '"').count() % 2 == 0;
    let sanitized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == '_' || (balanced && ch == '"') {
                ch
            } else {
                ' '
            }
        })
        .collect();
    let mut quoted = false;
    let mut token = String::new();
    let mut tokens = Vec::new();
    for ch in sanitized.chars().chain(std::iter::once(' ')) {
        if ch == '"' {
            if !token.trim().is_empty() {
                tokens.push(format!("\"{}\"", token.trim()));
            }
            token.clear();
            quoted = !quoted;
        } else if ch.is_whitespace() && !quoted {
            if !token.trim().is_empty() {
                tokens.push(format!("\"{}\"", token.trim()));
            }
            token.clear();
        } else {
            token.push(ch);
        }
    }
    if tokens.is_empty() {
        None
    } else {
        Some(tokens.join(" "))
    }
}
async fn search_cards(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Query(query): Query<SearchQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let page = query.page.unwrap_or(1).clamp(1, 102);
    let Some(term) = query.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(paginated(json!([]), 0, page, &uri, &headers));
    };
    let (offset, limit) = page_window(page);
    let number = term.parse::<i64>().ok();
    if let Some(number) = number {
        if let Ok(row) = card(&app, &p, &number.to_string()).await {
            let items = if offset == 0 {
                vec![card_json(&app, &p, &row, &base(&headers), false).await?]
            } else {
                vec![]
            };
            return Ok(paginated(json!(items), 1, page, &uri, &headers));
        }
    }
    let Some(search) = fts_query(term) else {
        return Ok(paginated(json!([]), 0, page, &uri, &headers));
    };
    let filter = " FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.account_id=? AND c.status='published' AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?) AND (c.rowid IN (SELECT rowid FROM card_search WHERE card_search MATCH ?) OR c.id IN (SELECT m.card_id FROM comments m WHERE m.rowid IN (SELECT rowid FROM comment_search WHERE comment_search MATCH ?)))";
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*){filter}"))
        .bind(&p.account_id)
        .bind(&p.user_id)
        .bind(&search)
        .bind(&search)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let rows = sqlx::query(&format!(
        "SELECT c.*{filter} ORDER BY c.last_active_at DESC,c.id DESC LIMIT ? OFFSET ?"
    ))
    .bind(&p.account_id)
    .bind(&p.user_id)
    .bind(&search)
    .bind(&search)
    .bind(limit)
    .bind(offset)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let items = cards_json_batch(&app, &p, &rows, &base(&headers)).await?;
    Ok(paginated(json!(items), total, page, &uri, &headers))
}
#[derive(Default)]
struct ActivityQuery {
    creators: Vec<String>,
    boards: Vec<String>,
    page: i64,
}
fn parse_activity_query(raw: Option<String>) -> ActivityQuery {
    let mut query = ActivityQuery {
        page: 1,
        ..Default::default()
    };
    if let Some(raw) = raw {
        for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
            match key.as_ref() {
                "creator_ids[]" => query.creators.push(value.into_owned()),
                "board_ids[]" => query.boards.push(value.into_owned()),
                "page" => query.page = value.parse::<i64>().unwrap_or(1).max(1),
                _ => {}
            }
        }
    }
    query
}
fn activities_query<'a>(
    select: &str,
    p: &'a Principal,
    q: &'a ActivityQuery,
) -> QueryBuilder<'a, Sqlite> {
    let mut builder = QueryBuilder::new(format!(
        "{select} FROM events e JOIN cards c ON c.id=e.card_id JOIN boards b ON b.id=c.board_id WHERE e.account_id="
    ));
    builder.push_bind(&p.account_id).push(" AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=")
        .push_bind(&p.user_id).push(") AND (e.eventable_type='Card' OR EXISTS (SELECT 1 FROM comments m WHERE m.id=e.eventable_id AND m.card_id=c.id))");
    for (field, values) in [("e.creator_id", &q.creators), ("c.board_id", &q.boards)] {
        if !values.is_empty() {
            builder.push(" AND ").push(field).push(" IN (");
            let mut separated = builder.separated(",");
            for value in values {
                separated.push_bind(value);
            }
            separated.push_unseparated(")");
        }
    }
    builder
}
async fn rows_for_ids(app: &App, prefix: &str, ids: &[String]) -> Result<Vec<SqliteRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(prefix);
    let mut separated = builder.separated(",");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    builder.build().fetch_all(&app.db).await.map_err(db_err)
}
async fn assignment_rows_for_cards(app: &App, ids: &[String]) -> Result<Vec<SqliteRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = QueryBuilder::<Sqlite>::new(
        "SELECT card_id,assignee_id FROM assignments WHERE card_id IN (",
    );
    let mut separated = builder.separated(",");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    builder.push(" ORDER BY created_at,id");
    builder.build().fetch_all(&app.db).await.map_err(db_err)
}
fn unique_ids(values: impl IntoIterator<Item = String>) -> Vec<String> {
    values
        .into_iter()
        .collect::<HashSet<_>>()
        .into_iter()
        .collect()
}
fn activity_description(
    p: &Principal,
    event: &SqliteRow,
    title: &str,
    users: &HashMap<String, Value>,
) -> String {
    let actor = users
        .get(&event.get::<String, _>("creator_id"))
        .and_then(|u| u["name"].as_str())
        .unwrap_or("");
    let quoted = format!("&quot;{}&quot;", escape(title));
    let action: String = event.get("action");
    let particulars: Value =
        serde_json::from_str(&event.get::<String, _>("particulars")).unwrap_or(json!({}));
    let text = match action.as_str() {
        "card_published" => format!("{actor} added {quoted}"),
        "card_closed" => format!("{actor} moved {quoted} to \"Done\""),
        "card_reopened" => format!("{actor} reopened {quoted}"),
        "card_postponed" => format!("{actor} moved {quoted} to \"Not Now\""),
        "card_auto_postponed" => format!("{quoted} moved to \"Not Now\" due to inactivity"),
        "card_sent_back_to_triage" => format!("{actor} moved {quoted} back to \"Maybe?\""),
        "card_triaged" => format!(
            "{actor} moved {quoted} to \"{}\"",
            escape(particulars["column"].as_str().unwrap_or(""))
        ),
        "card_title_changed" => format!(
            "{actor} renamed {quoted} (was: \"{}\")",
            escape(particulars["old_title"].as_str().unwrap_or(""))
        ),
        "card_board_changed" => format!(
            "{actor} moved {quoted} to \"{}\"",
            escape(particulars["new_board"].as_str().unwrap_or(""))
        ),
        "comment_created" => format!("{actor} commented on {quoted}"),
        "card_assigned" => {
            let ids = particulars["assignee_ids"].as_array();
            if ids.is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(&p.user_id))) {
                format!("{actor} will handle {quoted}")
            } else {
                let assignee = ids
                    .and_then(|ids| ids.first())
                    .and_then(Value::as_str)
                    .unwrap_or("");
                let name = users
                    .get(assignee)
                    .and_then(|u| u["name"].as_str())
                    .unwrap_or("");
                format!("{actor} assigned {name} to {quoted}")
            }
        }
        "card_unassigned" => {
            let ids = particulars["assignee_ids"].as_array();
            let assignee = ids
                .and_then(|ids| ids.first())
                .and_then(Value::as_str)
                .unwrap_or("");
            let target = if assignee == p.user_id {
                "yourself".to_string()
            } else {
                users
                    .get(assignee)
                    .and_then(|u| u["name"].as_str())
                    .unwrap_or("")
                    .to_string()
            };
            format!("{actor} unassigned {target} from {quoted}")
        }
        _ => String::new(),
    };
    text
}
async fn activity_items(
    app: &App,
    p: &Principal,
    events: &[SqliteRow],
    host: &str,
) -> Result<Vec<Value>> {
    if events.is_empty() {
        return Ok(Vec::new());
    }
    let card_ids = unique_ids(events.iter().map(|e| e.get::<String, _>("card_id")));
    let card_rows = rows_for_ids(app, "SELECT * FROM cards WHERE id IN (", &card_ids).await?;
    let board_ids = unique_ids(card_rows.iter().map(|c| c.get::<String, _>("board_id")));
    let board_rows = rows_for_ids(app, "SELECT * FROM boards WHERE id IN (", &board_ids).await?;
    let assignment_rows = assignment_rows_for_cards(app, &card_ids).await?;
    let tag_rows=rows_for_ids(app,"SELECT g.card_id,t.title FROM taggings g JOIN tags t ON t.id=g.tag_id WHERE g.card_id IN (",&card_ids).await?;
    let column_ids = unique_ids(
        card_rows
            .iter()
            .filter_map(|c| c.get::<Option<String>, _>("column_id")),
    );
    let column_rows = rows_for_ids(app, "SELECT * FROM columns WHERE id IN (", &column_ids).await?;
    let comment_ids = unique_ids(
        events
            .iter()
            .filter(|e| e.get::<String, _>("eventable_type") == "Comment")
            .map(|e| e.get::<String, _>("eventable_id")),
    );
    let comment_rows =
        rows_for_ids(app, "SELECT * FROM comments WHERE id IN (", &comment_ids).await?;
    let image_values = storage::image_urls_for_cards(app, p, &card_ids).await?;
    let user_ids = unique_ids(
        card_rows
            .iter()
            .map(|c| c.get::<String, _>("creator_id"))
            .chain(board_rows.iter().map(|b| b.get::<String, _>("creator_id")))
            .chain(
                assignment_rows
                    .iter()
                    .map(|a| a.get::<String, _>("assignee_id")),
            )
            .chain(
                comment_rows
                    .iter()
                    .map(|c| c.get::<String, _>("creator_id")),
            )
            .chain(events.iter().map(|e| e.get::<String, _>("creator_id"))),
    );
    let user_rows=rows_for_ids(app,"SELECT u.*,i.email_address FROM users u JOIN identities i ON i.id=u.identity_id WHERE u.id IN (",&user_ids).await?;
    let users: HashMap<String, Value> = user_rows
        .iter()
        .map(|u| (u.get::<String, _>("id"), user_json_row(p, u, host)))
        .collect();
    let mut boards = HashMap::<String, Value>::new();
    for b in &board_rows {
        let creator = b.get::<String, _>("creator_id");
        let user = users.get(&creator).cloned().ok_or_else(missing)?;
        boards.insert(b.get("id"), board_json_row(p, b, host, user));
    }
    let mut columns = HashMap::<String, Value>::new();
    for c in &column_rows {
        columns.insert(c.get("id"), column_json(p, c, host));
    }
    let mut tags = HashMap::<String, Vec<String>>::new();
    for t in &tag_rows {
        tags.entry(t.get("card_id"))
            .or_default()
            .push(t.get("title"));
    }
    for list in tags.values_mut() {
        list.sort();
    }
    let mut assignments = HashMap::<String, Vec<String>>::new();
    for a in &assignment_rows {
        assignments
            .entry(a.get("card_id"))
            .or_default()
            .push(a.get("assignee_id"));
    }
    let mut cards = HashMap::<String, Value>::new();
    let mut card_meta = HashMap::<String, (i64, String, String)>::new();
    for c in &card_rows {
        let id: String = c.get("id");
        let board_id: String = c.get("board_id");
        let creator_id: String = c.get("creator_id");
        let number: i64 = c.get("number");
        let title: String = c.get("title");
        let description: String = c.get("description");
        let (description_plain, description_html, has_attachments) =
            storage::render_rich_text(app, &p.account_id, p.slug, &description).await?;
        let assigned = assignments.get(&id).cloned().unwrap_or_default();
        let assignees: Vec<Value> = assigned
            .iter()
            .take(5)
            .filter_map(|id| users.get(id).cloned())
            .collect();
        let mut value = json!({"id":id,"number":number,"title":title,"status":c.get::<String,_>("status"),
            "description":description_plain,"description_html":description_html,"image_url":image_values.get(&id).cloned().unwrap_or(Value::Null),
            "has_attachments":has_attachments,"tags":tags.get(&id).cloned().unwrap_or_default(),
            "closed":c.get::<i64,_>("closed")!=0,"postponed":c.get::<i64,_>("postponed")!=0,
            "golden":c.get::<i64,_>("golden")!=0,"last_active_at":c.get::<String,_>("last_active_at"),
            "created_at":c.get::<String,_>("created_at"),"url":format!("{host}/{}/cards/{number}",p.slug),
            "board":boards.get(&board_id).cloned().ok_or_else(missing)?,
            "creator":users.get(&creator_id).cloned().ok_or_else(missing)?,
            "assignees":assignees,"has_more_assignees":assigned.len()>5,
            "comments_url":format!("{host}/{}/cards/{number}/comments",p.slug),
            "reactions_url":format!("{host}/{}/cards/{number}/reactions",p.slug)});
        if let Some(column_id) = c.get::<Option<String>, _>("column_id") {
            if let Some(column) = columns.get(&column_id) {
                value["column"] = column.clone();
            }
        }
        card_meta.insert(id.clone(), (number, title, board_id));
        cards.insert(id, value);
    }
    let mut comments = HashMap::<String, Value>::new();
    for c in &comment_rows {
        let id: String = c.get("id");
        let card_id: String = c.get("card_id");
        let (number, _, _) = card_meta.get(&card_id).ok_or_else(missing)?;
        let body: String = c.get("body");
        let (body_plain, body_html, _) =
            storage::render_rich_text(app, &p.account_id, p.slug, &body).await?;
        let created: String = c.get("created_at");
        let updated = c
            .get::<Option<String>, _>("updated_at")
            .unwrap_or_else(|| created.clone());
        let creator = users
            .get(&c.get::<String, _>("creator_id"))
            .cloned()
            .ok_or_else(missing)?;
        comments.insert(
            id.clone(),
            json!({"id":id,"created_at":created,"updated_at":updated,
            "body":{"plain_text":body_plain,"html":body_html},"creator":creator,
            "card":{"id":card_id,"url":format!("{host}/{}/cards/{number}",p.slug)},
            "reactions_url":format!("{host}/{}/cards/{number}/comments/{id}/reactions",p.slug),
            "url":format!("{host}/{}/cards/{number}/comments/{id}",p.slug)}),
        );
    }
    let mut items = Vec::with_capacity(events.len());
    for e in events {
        let card_id: String = e.get("card_id");
        let (number, title, board_id) = card_meta.get(&card_id).ok_or_else(missing)?;
        let eventable_id: String = e.get("eventable_id");
        let eventable_type: String = e.get("eventable_type");
        let (eventable, url) = if eventable_type == "Comment" {
            (
                comments.get(&eventable_id).cloned().ok_or_else(missing)?,
                format!("{host}/{}/cards/{number}#comment_{eventable_id}", p.slug),
            )
        } else {
            (
                cards.get(&card_id).cloned().ok_or_else(missing)?,
                format!("{host}/{}/cards/{number}", p.slug),
            )
        };
        let particulars: Value =
            serde_json::from_str(&e.get::<String, _>("particulars")).unwrap_or(json!({}));
        items.push(
            json!({"id":e.get::<String,_>("id"),"action":e.get::<String,_>("action"),
            "created_at":e.get::<String,_>("created_at"),
            "description":activity_description(p,e,title,&users),"particulars":particulars,
            "url":url,"eventable_type":eventable_type,"eventable":eventable,
            "board":boards.get(board_id).cloned().ok_or_else(missing)?,
            "creator":users.get(&e.get::<String,_>("creator_id")).cloned().ok_or_else(missing)?}),
        );
    }
    Ok(items)
}
async fn show_notification_settings(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let frequency = sqlx::query_scalar::<_, String>(
        "SELECT bundle_email_frequency FROM notification_settings WHERE user_id=? AND account_id=?",
    )
    .bind(&p.user_id)
    .bind(&p.account_id)
    .fetch_optional(&app.db)
    .await
    .map_err(db_err)?
    .unwrap_or_else(|| "every_few_hours".into());
    if headers.contains_key("x-rizzy-json-request") {
        return Ok(Json(json!({"bundle_email_frequency":frequency})).into_response());
    }
    let boards = sqlx::query("SELECT b.id,b.name,a.involvement FROM accesses a JOIN boards b ON b.id=a.board_id WHERE a.user_id=? AND b.account_id=? ORDER BY lower(b.name)")
        .bind(&p.user_id).bind(&p.account_id).fetch_all(&app.db).await.map_err(db_err)?;
    let mut board_html = String::new();
    for board in boards {
        let id: String = board.get("id");
        let name: String = board.get("name");
        let involvement: String = board.get("involvement");
        let watching = involvement == "watching";
        let action = if watching {
            "Stop watching"
        } else {
            "Watch this"
        };
        board_html.push_str(&format!(
            "<div class=\"board-involvement\"><h3>{}</h3><span class=\"board-separator\" aria-hidden=\"true\"></span><button type=\"button\" class=\"watch-button{}\" data-board-involvement data-url=\"/{}/boards/{}/involvement\" data-involvement=\"{}\" aria-label=\"{}\" title=\"{}\" aria-pressed=\"{}\"><span class=\"watch-icon\" aria-hidden=\"true\">{}{}</span></button></div>",
            escape(&name),
            if watching { " is-watching" } else { "" },
            p.slug,
            escape(&id),
            involvement,
            action,
            action,
            watching,
            include_str!("../assets/notification-bell.svg"),
            include_str!("../assets/notification-bell-off.svg"),
        ));
    }
    let choices = [
        ("never", "Never"),
        ("every_few_hours", "Every few hours"),
        ("daily", "Every day"),
        ("weekly", "Every week"),
    ]
    .iter()
    .map(|(value, label)| {
        format!(
            "<option value=\"{value}\"{}>{label}</option>",
            if *value == frequency { " selected" } else { "" }
        )
    })
    .collect::<String>();
    let push_html = if let Some(key) = push_delivery::public_key() {
        format!(
            "<div data-push-settings data-vapid-key=\"{}\" data-subscriptions-url=\"/{}/users/{}/push_subscriptions\"><h2>Push notifications are <span data-push-state>OFF</span></h2><button type=\"button\">Turn ON push notifications on this device</button><p data-push-error role=\"status\"></p></div><script>{}</script>",
            escape(&key),
            p.slug,
            escape(&p.user_id),
            include_str!("../assets/notifications_push.js")
        )
    } else {
        "<h2>Push notifications are OFF</h2><p>Push delivery is not configured.</p>".into()
    };
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Notification Settings | Rizzy</title><style>:root{{font:16px/1.5 system-ui,sans-serif;background:#f7f5f0;color:#252322}}main{{max-width:900px;margin:35px auto;padding:20px}}.panels{{display:grid;grid-template-columns:1fr 1fr;gap:24px}}section{{background:white;padding:22px;border-radius:12px;box-shadow:0 4px 24px #0001}}select,button{{font:inherit;padding:9px}}.board-involvement{{display:flex;align-items:center;gap:12px;min-height:42px}}.board-involvement h3{{font-size:1rem;font-weight:500;margin:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}}.board-separator{{flex:1;border-top:1px dashed #a6a19b}}.watch-button{{border:1px solid #bcb7b0;border-radius:50%;width:36px;height:36px;padding:0;background:white;cursor:pointer}}.watch-button.is-watching{{background:#252322;color:white;border-color:#252322}}.watch-icon svg{{width:20px;height:20px;fill:currentColor;vertical-align:middle}}.watch-icon svg:first-child{{display:none}}.is-watching .watch-icon svg:first-child{{display:inline}}.is-watching .watch-icon svg:last-child{{display:none}}.watch-button:focus-visible{{outline:3px solid #147dbe;outline-offset:2px}}.watch-button:disabled{{opacity:.5;cursor:wait}}@media(max-width:700px){{.panels{{grid-template-columns:1fr}}}}</style></head><body><main><p><a href=\"/{}/\">← Activity</a></p><h1>Notification Settings</h1><div class=\"panels\"><section><h2>Boards</h2><div class=\"board-list\">{board_html}</div><p id=\"board-involvement-error\" role=\"status\"></p></section><section>{push_html}<p><a href=\"/{}/users/{}/push_subscriptions\">Manage this device’s push subscriptions</a></p><h2>Email Notifications</h2><p>Get a single email with all your notifications every few hours, daily, or weekly.</p><form method=\"post\" action=\"/{}/notifications/settings\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><label for=\"bundle-email-frequency\">Email me about new notifications...</label><select id=\"bundle-email-frequency\" name=\"user_settings[bundle_email_frequency]\" onchange=\"this.form.requestSubmit()\">{choices}</select><button type=\"submit\">Save</button></form></section></div></main><script>{}</script></body></html>",
        p.slug,
        p.slug,
        escape(&p.user_id),
        p.slug,
        include_str!("../assets/notifications_boards.js")
    );
    Ok(Html(html).into_response())
}
async fn update_notification_settings(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"));
    let frequency = if form {
        url::form_urlencoded::parse(&body)
            .find(|(key, _)| key == "user_settings[bundle_email_frequency]")
            .map(|(_, value)| value.into_owned())
            .ok_or_else(|| bad("Missing email frequency"))?
    } else {
        let body: Value = serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?;
        required(nested(&body, "user_settings"), "bundle_email_frequency")?
    };
    if !["never", "every_few_hours", "daily", "weekly"].contains(&frequency.as_str()) {
        return Err(bad("Invalid bundle email frequency"));
    }
    let previous: Option<String> = sqlx::query_scalar(
        "SELECT bundle_email_frequency FROM notification_settings WHERE user_id=?",
    )
    .bind(&p.user_id)
    .fetch_optional(&app.db)
    .await
    .map_err(db_err)?;
    sqlx::query("INSERT INTO notification_settings(user_id,account_id,bundle_email_frequency) VALUES(?,?,?) ON CONFLICT(user_id) DO UPDATE SET bundle_email_frequency=excluded.bundle_email_frequency")
        .bind(&p.user_id).bind(&p.account_id).bind(&frequency).execute(&app.db).await.map_err(db_err)?;
    if previous.as_deref() != Some(frequency.as_str()) {
        if frequency == "never" {
            sqlx::query("DELETE FROM notification_bundles WHERE user_id=? AND status=0")
                .bind(&p.user_id)
                .execute(&app.db)
                .await
                .map_err(db_err)?;
        } else {
            sqlx::query("UPDATE notification_bundles SET ends_at=?,updated_at=? WHERE user_id=? AND status=0")
                .bind(now()).bind(now()).bind(&p.user_id)
                .execute(&app.db).await.map_err(db_err)?;
        }
    }
    if frequency != "never" {
        unsubscribes::ensure_token(&app, &p.user_id).await?;
    }
    if headers.contains_key("x-rizzy-json-request") {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("/{}/notifications/settings", p.slug),
            )],
        )
            .into_response())
    }
}

async fn notification_json(app: &App, p: &Principal, row: &SqliteRow, host: &str) -> Result<Value> {
    let card_id: String = row.get("card_id");
    let card = sqlx::query(
        "SELECT c.*,b.name board_name FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.id=?",
    )
    .bind(&card_id)
    .fetch_one(&app.db)
    .await
    .map_err(db_err)?;
    let creator_id: String = row.get("creator_id");
    let creator = user_json(app, p, &creator_id, host).await?;
    let column = if let Some(column_id) = card.get::<Option<String>, _>("column_id") {
        sqlx::query("SELECT * FROM columns WHERE id=?")
            .bind(column_id)
            .fetch_optional(&app.db)
            .await
            .map_err(db_err)?
            .map(|column| column_json(p, &column, host))
    } else {
        None
    };
    let card_title = card.get::<String, _>("title");
    let card_title = if card_title.trim().is_empty() {
        format!("Card {}", card.get::<i64, _>("number"))
    } else {
        card_title
    };
    let name = creator["name"].as_str().unwrap_or("");
    let source_kind: String = row.get("source_type");
    let (source_type, title, body) = if source_kind == "Mention" {
        let mention =
            sqlx::query("SELECT source_type,source_id FROM mentions WHERE id=? AND account_id=?")
                .bind(row.get::<String, _>("source_id"))
                .bind(&p.account_id)
                .fetch_one(&app.db)
                .await
                .map_err(db_err)?;
        let source_body = match mention.get::<String, _>("source_type").as_str() {
            "Card" => card.get::<String, _>("description"),
            "Comment" => sqlx::query_scalar::<_, String>("SELECT body FROM comments WHERE id=?")
                .bind(mention.get::<String, _>("source_id"))
                .fetch_one(&app.db)
                .await
                .map_err(db_err)?,
            _ => {
                return Err(err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Unsupported mention source",
                ));
            }
        };
        let first = name.split_whitespace().next().unwrap_or(name);
        let (plain_body, _, _) =
            storage::render_rich_text(app, &p.account_id, p.slug, &source_body).await?;
        (
            "mention",
            format!("{first} @mentioned you"),
            plain_body.chars().take(200).collect::<String>(),
        )
    } else {
        if source_kind != "Event" {
            return Err(err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unsupported notification source",
            ));
        }
        let source = sqlx::query("SELECT * FROM events WHERE id=?")
            .bind(row.get::<String, _>("source_id"))
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
        let action: String = source.get("action");
        let particulars: Value =
            serde_json::from_str(&source.get::<String, _>("particulars")).unwrap_or(json!({}));
        let body = match action.as_str() {
            "card_assigned" => {
                let assignee = particulars["assignee_ids"]
                    .as_array()
                    .and_then(|ids| ids.first())
                    .and_then(Value::as_str);
                if let Some(assignee) = assignee {
                    let assignee_name = sqlx::query_scalar::<_, String>(
                        "SELECT name FROM users WHERE id=? AND account_id=?",
                    )
                    .bind(assignee)
                    .bind(&p.account_id)
                    .fetch_optional(&app.db)
                    .await
                    .map_err(db_err)?;
                    format!(
                        "Assigned to {}",
                        assignee_name.unwrap_or_else(|| "self".into())
                    )
                } else {
                    "Assigned to self".into()
                }
            }
            "card_published" => format!("Added by {name}"),
            "card_closed" => format!("Moved to Done by {name}"),
            "card_reopened" => format!("Reopened by {name}"),
            "card_postponed" => format!("Moved to Not Now by {name}"),
            "card_unassigned" => format!("Unassigned by {name}"),
            "card_board_changed" => format!("Moved by {name}"),
            "card_sent_back_to_triage" => format!("Moved back to Maybe? by {name}"),
            "comment_created" => {
                let comment =
                    sqlx::query_scalar::<_, String>("SELECT body FROM comments WHERE id=?")
                        .bind(source.get::<String, _>("eventable_id"))
                        .fetch_optional(&app.db)
                        .await
                        .map_err(db_err)?;
                comment.unwrap_or_default().chars().take(200).collect()
            }
            _ => name.to_owned(),
        };
        let title = if action == "comment_created" {
            format!("RE: {card_title}")
        } else {
            card_title
        };
        ("event", title, body)
    };
    let number: i64 = card.get("number");
    let read_at: Option<String> = row.get("read_at");
    Ok(
        json!({"id":row.get::<String,_>("id"),"unread_count":row.get::<i64,_>("unread_count"),
        "read":read_at.is_some(),"read_at":read_at,"created_at":row.get::<String,_>("created_at"),
        "source_type":source_type,"title":title,
        "body":body,"creator":creator,"card":{"id":card_id,"number":number,
        "title":card.get::<String,_>("title"),"status":card.get::<String,_>("status"),
        "board_name":card.get::<String,_>("board_name"),"closed":card.get::<i64,_>("closed")!=0,
        "postponed":card.get::<i64,_>("postponed")!=0,
        "url":format!("{host}/{}/cards/{number}",p.slug),"column":column},
        "url":format!("{host}/{}/notifications/{}",p.slug,row.get::<String,_>("id"))}),
    )
}

async fn notification_items(
    app: &App,
    p: &Principal,
    rows: Vec<SqliteRow>,
    host: &str,
) -> Result<Vec<Value>> {
    let mut result = Vec::with_capacity(rows.len());
    for row in &rows {
        result.push(notification_json(app, p, row, host).await?);
    }
    Ok(result)
}

async fn list_notifications(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let unread=sqlx::query("SELECT * FROM notifications WHERE user_id=? AND account_id=? AND read_at IS NULL ORDER BY updated_at DESC LIMIT 100")
        .bind(&p.user_id).bind(&p.account_id).fetch_all(&app.db).await.map_err(db_err)?;
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let total:i64=sqlx::query_scalar("SELECT COUNT(*) FROM notifications WHERE user_id=? AND account_id=? AND read_at IS NOT NULL")
        .bind(&p.user_id).bind(&p.account_id).fetch_one(&app.db).await.map_err(db_err)?;
    let read=sqlx::query("SELECT * FROM notifications WHERE user_id=? AND account_id=? AND read_at IS NOT NULL ORDER BY read_at DESC,updated_at DESC LIMIT ? OFFSET ?")
        .bind(&p.user_id).bind(&p.account_id).bind(limit).bind(offset).fetch_all(&app.db).await.map_err(db_err)?;
    let host = base(&headers);
    let mut items = if page == 1 {
        notification_items(&app, &p, unread, &host).await?
    } else {
        Vec::new()
    };
    items.extend(notification_items(&app, &p, read, &host).await?);
    Ok(paginated(json!(items), total, page, &uri, &headers))
}

#[derive(Deserialize)]
struct NotificationTrayQuery {
    include_read: Option<bool>,
}
async fn notification_tray(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Query(query): Query<NotificationTrayQuery>,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    let unread=sqlx::query("SELECT * FROM notifications WHERE user_id=? AND account_id=? AND read_at IS NULL ORDER BY updated_at DESC LIMIT 100")
        .bind(&p.user_id).bind(&p.account_id).fetch_all(&app.db).await.map_err(db_err)?;
    let host = base(&headers);
    let mut items = notification_items(&app, &p, unread, &host).await?;
    if query.include_read.unwrap_or(false) {
        let read=sqlx::query("SELECT * FROM notifications WHERE user_id=? AND account_id=? AND read_at IS NOT NULL ORDER BY read_at DESC,updated_at DESC LIMIT 100")
            .bind(&p.user_id).bind(&p.account_id).fetch_all(&app.db).await.map_err(db_err)?;
        items.extend(notification_items(&app, &p, read, &host).await?);
    }
    Ok(Json(json!(items)))
}
async fn read_notification(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let result=sqlx::query("UPDATE notifications SET read_at=?,unread_count=0,updated_at=? WHERE id=? AND user_id=? AND account_id=?")
        .bind(now()).bind(now()).bind(id.trim_end_matches(".json")).bind(&p.user_id).bind(&p.account_id)
        .execute(&app.db).await.map_err(db_err)?;
    if result.rows_affected() == 0 {
        return Err(missing());
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn unread_notification(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let result=sqlx::query("UPDATE notifications SET read_at=NULL,unread_count=1,updated_at=? WHERE id=? AND user_id=? AND account_id=?")
        .bind(now()).bind(id.trim_end_matches(".json")).bind(&p.user_id).bind(&p.account_id)
        .execute(&app.db).await.map_err(db_err)?;
    if result.rows_affected() == 0 {
        return Err(missing());
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn read_all_notifications(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    sqlx::query("UPDATE notifications SET read_at=?,unread_count=0,updated_at=? WHERE user_id=? AND account_id=? AND read_at IS NULL")
        .bind(now()).bind(now()).bind(&p.user_id).bind(&p.account_id).execute(&app.db).await.map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_activities(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let query = parse_activity_query(raw);
    let (offset, limit) = page_window(query.page);
    let mut count = activities_query("SELECT COUNT(*)", &p, &query);
    let total: i64 = count
        .build_query_scalar()
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let mut builder = activities_query("SELECT e.*", &p, &query);
    builder
        .push(" ORDER BY e.created_at DESC,e.id DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
    let rows = builder.build().fetch_all(&app.db).await.map_err(db_err)?;
    let host = base(&headers);
    let items = activity_items(&app, &p, &rows, &host).await?;
    Ok(paginated(json!(items), total, query.page, &uri, &headers))
}
async fn resolved_card_query(app: &App, p: &Principal, raw: Option<String>) -> Result<CardQuery> {
    let mut query = parse_card_query(raw);
    if let Some(filter_id) = query.filter_id.as_deref() {
        let fields: String = sqlx::query_scalar(
            "SELECT fields FROM filters WHERE id=? AND account_id=? AND creator_id=?",
        )
        .bind(filter_id)
        .bind(&p.account_id)
        .bind(&p.user_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)?;
        let stored: Value = serde_json::from_str(&fields)
            .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Invalid saved filter"))?;
        query = card_query_from_filter(&stored, query.page);
    }
    Ok(query)
}
async fn list_cards(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let query = resolved_card_query(&app, &p, raw.clone()).await?;
    let (offset, limit) = page_window(query.page);
    let order = match query.sorted_by.as_deref() {
        Some("oldest") => "c.created_at ASC",
        Some("newest") => "c.created_at DESC",
        _ => "c.last_active_at DESC",
    };
    let mut count = cards_query("SELECT COUNT(*)", &p, &query);
    let total: i64 = count
        .build_query_scalar()
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let html = !headers.contains_key("x-rizzy-json-request");
    let select = if html {
        "SELECT c.*,b.name AS board_name,(SELECT name FROM columns WHERE id=c.column_id) AS column_name,(SELECT color FROM columns WHERE id=c.column_id) AS column_color,(SELECT name FROM users WHERE id=c.creator_id) AS creator_name"
    } else {
        "SELECT c.*,b.name AS board_name"
    };
    let mut builder = cards_query(select, &p, &query);
    builder
        .push(format!(" ORDER BY {order},c.id DESC LIMIT "))
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
    let rows = builder.build().fetch_all(&app.db).await.map_err(db_err)?;
    if html {
        return Ok(
            cards_page::show(&app, &p, &rows, total, &query, raw.as_deref())
                .await?
                .into_response(),
        );
    }
    let out = cards_json_batch(&app, &p, &rows, &base(&headers)).await?;
    Ok(paginated(json!(out), total, query.page, &uri, &headers))
}
async fn card_previews(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let accepts_stream = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/vnd.turbo-stream.html"));
    if !uri.path().ends_with(".turbo_stream") && !accepts_stream {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let p = principal(&app, &headers, &account).await?;
    let target = raw
        .as_deref()
        .and_then(|raw| {
            url::form_urlencoded::parse(raw.as_bytes())
                .find(|(key, _)| key == "target")
                .map(|(_, value)| value.into_owned())
        })
        .unwrap_or_default();
    let query = resolved_card_query(&app, &p, raw.clone()).await?;
    let (offset, limit) = page_window(query.page);
    let order = match query.sorted_by.as_deref() {
        Some("oldest") => "c.created_at ASC",
        Some("newest") => "c.created_at DESC",
        _ => "c.last_active_at DESC",
    };
    let mut count = cards_query("SELECT COUNT(*)", &p, &query);
    let total: i64 = count
        .build_query_scalar()
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let mut builder = cards_query(
        "SELECT c.*,b.name AS board_name,(SELECT name FROM columns WHERE id=c.column_id) AS column_name,(SELECT color FROM columns WHERE id=c.column_id) AS column_color,(SELECT name FROM users WHERE id=c.creator_id) AS creator_name",
        &p,
        &query,
    );
    builder
        .push(format!(" ORDER BY {order},c.id DESC LIMIT "))
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
    let rows = builder.build().fetch_all(&app.db).await.map_err(db_err)?;
    let mut cards = cards_page::previews(&app, &p, &rows).await?;
    if offset + (rows.len() as i64) < total {
        cards.push_str(&cards_page::next_link(
            &p,
            &target,
            query.page + 1,
            raw.as_deref(),
        ));
    }
    let html = format!(
        "<turbo-stream action=\"remove\" target=\"{}-load-page-{}\"></turbo-stream><turbo-stream action=\"append\" target=\"{}\"><template>{cards}</template></turbo-stream>",
        escape(&target),
        query.page,
        escape(&target)
    );
    Ok((
        [(
            header::CONTENT_TYPE,
            "text/vnd.turbo-stream.html; charset=utf-8",
        )],
        html,
    )
        .into_response())
}
fn filter_params(headers: &HeaderMap, body: &Bytes) -> Result<Value> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded");
    let raw: Value = if form {
        let mut fields = serde_json::Map::new();
        for (key, value) in url::form_urlencoded::parse(body) {
            let key = if let Some(inner) = key.strip_prefix("filter[") {
                if let Some(inner) = inner.strip_suffix("][]") {
                    format!("{inner}[]")
                } else {
                    inner.strip_suffix(']').unwrap_or(inner).to_owned()
                }
            } else {
                key.into_owned()
            };
            if let Some(key) = key.strip_suffix("[]") {
                let entry = fields.entry(key.to_string()).or_insert_with(|| json!([]));
                if let Some(array) = entry.as_array_mut() {
                    array.push(json!(value.into_owned()));
                } else {
                    *entry = json!([value.into_owned()]);
                }
            } else {
                fields.insert(key.to_string(), json!(value.into_owned()));
            }
        }
        Value::Object(fields)
    } else {
        serde_json::from_slice(body).map_err(|_| bad("Invalid JSON"))?
    };
    let input = raw.get("filter").unwrap_or(&raw);
    let mut params = serde_json::Map::new();
    for key in [
        "assignment_status",
        "indexed_by",
        "sorted_by",
        "creation",
        "closure",
    ] {
        let Some(value) = input
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        else {
            continue;
        };
        if (key == "indexed_by" && value == "all") || (key == "sorted_by" && value == "latest") {
            continue;
        }
        params.insert(key.into(), json!(value));
    }
    for key in [
        "card_ids",
        "column_ids",
        "assignee_ids",
        "creator_ids",
        "closer_ids",
        "board_ids",
        "tag_ids",
        "terms",
    ] {
        let Some(values) = input.get(key).and_then(Value::as_array) else {
            continue;
        };
        let mut normalized = Vec::new();
        for value in values {
            let Some(value) = value.as_str().filter(|v| !v.is_empty()) else {
                continue;
            };
            if key == "column_ids" || key.ends_with("_ids") && key != "card_ids" {
                if normalized.iter().any(|item: &String| item == value) {
                    continue;
                }
            }
            normalized.push(value.to_owned());
        }
        if !normalized.is_empty() {
            params.insert(key.into(), json!(normalized));
        }
    }
    Ok(Value::Object(params))
}
fn filter_turbo(inner: String) -> Response {
    let html = format!(
        "<turbo-stream action=\"replace\" target=\"filter-settings-save-toggle\"><template><div id=\"filter-settings-save-toggle\">{inner}</div></template></turbo-stream>"
    );
    (
        [(
            header::CONTENT_TYPE,
            "text/vnd.turbo-stream.html; charset=utf-8",
        )],
        html,
    )
        .into_response()
}
fn filter_save_button(slug: i64, params: &Value) -> String {
    let mut inputs = String::new();
    if let Some(fields) = params.as_object() {
        for (key, value) in fields {
            if let Some(values) = value.as_array() {
                for item in values {
                    if let Some(item) = item.as_str() {
                        inputs.push_str(&format!(
                            "<input type=hidden name=\"{}[]\" value=\"{}\">",
                            escape(key),
                            escape(item)
                        ));
                    }
                }
            } else if let Some(value) = value.as_str() {
                inputs.push_str(&format!(
                    "<input type=hidden name=\"{}\" value=\"{}\">",
                    escape(key),
                    escape(value)
                ));
            }
        }
    }
    format!(
        "<form method=post action=\"/{slug}/filters\">{inputs}<button type=submit aria-label=\"Save custom view\">Save custom view</button></form>"
    )
}
async fn refresh_filter_settings(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let filter_id = if headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .starts_with("application/x-www-form-urlencoded")
    {
        url::form_urlencoded::parse(&body)
            .find(|(key, _)| key == "filter_id")
            .map(|(_, value)| value.into_owned())
    } else {
        serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|value| {
                value
                    .get("filter_id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
            })
    };
    let fields = if let Some(filter_id) = filter_id.filter(|value| !value.is_empty()) {
        sqlx::query_scalar::<_, String>(
            "SELECT fields FROM filters WHERE id=? AND account_id=? AND creator_id=?",
        )
        .bind(filter_id)
        .bind(&p.account_id)
        .bind(&p.user_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)?
    } else {
        filter_params(&headers, &body)?.to_string()
    };
    let digest = hash(&fields);
    let saved_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM filters WHERE account_id=? AND creator_id=? AND params_digest=?",
    )
    .bind(&p.account_id)
    .bind(&p.user_id)
    .bind(digest)
    .fetch_optional(&app.db)
    .await
    .map_err(db_err)?;
    Ok(if let Some(saved_id) = saved_id {
        filter_turbo(format!(
            "<form method=post action=\"/{}/filters/{}\"><input type=hidden name=_method value=delete><button type=submit aria-label=\"Delete custom view\">Delete custom view</button></form>",
            p.slug, saved_id
        ))
    } else {
        let params: Value = serde_json::from_str(&fields)
            .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Invalid saved filter"))?;
        filter_turbo(filter_save_button(p.slug, &params))
    })
}
async fn validate_filter_resources(app: &App, p: &Principal, params: &Value) -> Result<()> {
    for (key, sql) in [
        (
            "tag_ids",
            "SELECT EXISTS(SELECT 1 FROM tags WHERE id=? AND account_id=?)",
        ),
        (
            "board_ids",
            "SELECT EXISTS(SELECT 1 FROM boards b JOIN accesses a ON a.board_id=b.id WHERE b.id=? AND b.account_id=? AND a.user_id=?)",
        ),
        (
            "assignee_ids",
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND account_id=?)",
        ),
        (
            "creator_ids",
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND account_id=?)",
        ),
        (
            "closer_ids",
            "SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND account_id=?)",
        ),
    ] {
        if let Some(ids) = params.get(key).and_then(Value::as_array) {
            for id in ids.iter().filter_map(Value::as_str) {
                let mut query = sqlx::query_scalar::<_, i64>(sql)
                    .bind(id)
                    .bind(&p.account_id);
                if key == "board_ids" {
                    query = query.bind(&p.user_id);
                }
                if query.fetch_one(&app.db).await.map_err(db_err)? == 0 {
                    return Err(missing());
                }
            }
        }
    }
    Ok(())
}
async fn create_filter(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let params = filter_params(&headers, &body)?;
    validate_filter_resources(&app, &p, &params).await?;
    let fields = params.to_string();
    let digest = hash(&fields);
    let filter_id = id();
    let timestamp = now();
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let saved_id: String = sqlx::query_scalar("INSERT INTO filters(id,account_id,creator_id,fields,params_digest,created_at,updated_at) VALUES(?,?,?,?,?,?,?) ON CONFLICT(creator_id,params_digest) DO UPDATE SET updated_at=excluded.updated_at RETURNING id")
        .bind(filter_id).bind(&p.account_id).bind(&p.user_id).bind(&fields).bind(digest)
        .bind(&timestamp).bind(&timestamp).fetch_one(&mut *tx).await.map_err(db_err)?;
    for (key, table, column) in [
        ("tag_ids", "filters_tags", "tag_id"),
        ("board_ids", "boards_filters", "board_id"),
        ("assignee_ids", "assignees_filters", "assignee_id"),
        ("creator_ids", "creators_filters", "creator_id"),
        ("closer_ids", "closers_filters", "closer_id"),
    ] {
        if let Some(ids) = params.get(key).and_then(Value::as_array) {
            let statement =
                format!("INSERT OR IGNORE INTO {table}(filter_id,{column}) VALUES(?,?)");
            for resource_id in ids.iter().filter_map(Value::as_str) {
                sqlx::query(&statement)
                    .bind(&saved_id)
                    .bind(resource_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(db_err)?;
            }
        }
    }
    tx.commit().await.map_err(db_err)?;
    Ok(filter_turbo(format!(
        "<form method=post action=\"/{}/filters/{}\"><input type=hidden name=_method value=delete><button type=submit aria-label=\"Delete custom view\">Delete custom view</button></form>",
        p.slug, saved_id
    )))
}
async fn delete_filter(
    State(app): Shared,
    Path((account, filter_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let fields: String = sqlx::query_scalar(
        "SELECT fields FROM filters WHERE id=? AND creator_id=? AND account_id=?",
    )
    .bind(&filter_id)
    .bind(&p.user_id)
    .bind(&p.account_id)
    .fetch_optional(&app.db)
    .await
    .map_err(db_err)?
    .ok_or_else(missing)?;
    let result = sqlx::query("DELETE FROM filters WHERE id=? AND creator_id=? AND account_id=?")
        .bind(&filter_id)
        .bind(&p.user_id)
        .bind(&p.account_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    if result.rows_affected() == 0 {
        return Err(missing());
    }
    let params = serde_json::from_str(&fields)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Invalid saved filter"))?;
    Ok(filter_turbo(filter_save_button(p.slug, &params)))
}
async fn column_cards(
    State(app): Shared,
    Path((account, board_id, column_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    checked_column(&app, &p, &board_id, &column_id).await?;
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let scope = " FROM cards WHERE board_id=? AND column_id=? AND status='published' AND closed=0 AND postponed=0";
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*){scope}"))
        .bind(&board_id)
        .bind(&column_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let rows = sqlx::query(&format!(
        "SELECT c.*,(SELECT name FROM boards WHERE id=c.board_id) AS board_name,(SELECT name FROM columns WHERE id=c.column_id) AS column_name,(SELECT color FROM columns WHERE id=c.column_id) AS column_color,(SELECT name FROM users WHERE id=c.creator_id) AS creator_name FROM cards c WHERE c.board_id=? AND c.column_id=? AND c.status='published' AND c.closed=0 AND c.postponed=0 ORDER BY c.golden DESC,c.last_active_at DESC,c.id DESC LIMIT ? OFFSET ?"
    ))
    .bind(&board_id)
    .bind(&column_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    if headers.contains_key("x-rizzy-card-fragment") {
        return Ok(Html(cards_page::previews(&app, &p, &rows).await?).into_response());
    }
    let out = cards_json_batch(&app, &p, &rows, &base(&headers)).await?;
    Ok(paginated(json!(out), total, page, &uri, &headers))
}
async fn show_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let c = card(&app, &p, &number).await?;
    if c.get::<String, _>("status") == "drafted" {
        return Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("{}/{account}/cards/{number}/draft", base(&headers)),
            )],
        )
            .into_response());
    }
    if !headers.contains_key("x-rizzy-json-request") {
        return Ok(card_page::show(&app, &p, &c).await?.into_response());
    }
    Ok(Json(card_json(&app, &p, &c, &base(&headers), true).await?).into_response())
}
async fn edit_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let p = principal(&app, &headers, &account).await?;
    let c = card(&app, &p, &number).await?;
    Ok(card_page::edit(&p, &c).into_response())
}
async fn create_card(
    State(app): Shared,
    Path((account, board_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    board(&app, &p, &board_id).await?;
    if !headers.contains_key("x-rizzy-json-request") {
        return create_or_resume_draft(&app, &p, &account, &board_id, &headers, false).await;
    }
    let body: Value =
        serde_json::from_slice(&body).map_err(|_| err(StatusCode::BAD_REQUEST, "Invalid JSON"))?;
    let v = nested(&body, "card");
    let title = string(v, "title")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Untitled".into());
    let description = string(v, "description").unwrap_or_default();
    let (plain_description, _, _) =
        storage::render_rich_text(&app, &p.account_id, p.slug, &description).await?;
    let search_text = search_content(plain_description);
    let event_id = id();
    let id = id();
    let timestamp = now();
    let created_at = timestamp_param(v, "created_at")?.unwrap_or_else(|| timestamp.clone());
    let last_active_at =
        timestamp_param(v, "last_active_at")?.unwrap_or_else(|| created_at.clone());
    let writer = app.write_lock.lock().await;
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let number: i64 = sqlx::query_scalar(
        "UPDATE accounts SET cards_count=cards_count+1 WHERE id=? RETURNING cards_count",
    )
    .bind(&p.account_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_err)?;
    sqlx::query("INSERT INTO cards(id,account_id,board_id,creator_id,number,title,description,search_text,created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
        .bind(&id).bind(&p.account_id).bind(&board_id).bind(&p.user_id).bind(number).bind(title).bind(&description).bind(&search_text).bind(&created_at).bind(&timestamp).bind(&last_active_at).execute(&mut *tx).await.map_err(db_err)?;
    storage::attach_card_image_tx(&mut tx, &p, &id, v.get("image")).await?;
    storage::sync_embeds_tx(&mut tx, &p, "Card", &id, &description).await?;
    sqlx::query("INSERT INTO watches(card_id,user_id,watching) VALUES(?,?,1)")
        .bind(&id)
        .bind(&p.user_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,particulars,created_at) VALUES(?,?,?,?,?,'Card',?,'card_published','{}',?)")
        .bind(&event_id).bind(&p.account_id).bind(&board_id).bind(&id).bind(&p.user_id)
        .bind(&id).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
    queue_webhooks(
        &mut tx,
        &p.account_id,
        &board_id,
        &event_id,
        "card_published",
        &timestamp,
    )
    .await?;
    queue_notifications(
        &mut tx,
        &p,
        &board_id,
        &id,
        &event_id,
        "Card",
        "card_published",
        &json!({}),
        &timestamp,
    )
    .await?;
    create_new_mentions(&mut tx, &p, &board_id, &id, "Card", &id, &description).await?;
    tx.commit().await.map_err(db_err)?;
    drop(writer);
    let c = card(&app, &p, &number.to_string()).await?;
    Ok(created(
        format!("/{account}/cards/{number}.json"),
        card_json(&app, &p, &c, &base(&headers), true).await?,
    ))
}
async fn create_account_card(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    body: Bytes,
) -> Result<Response> {
    let query_board = raw_query.as_deref().and_then(|raw| {
        url::form_urlencoded::parse(raw.as_bytes())
            .find(|(key, _)| key == "board_id")
            .map(|(_, value)| value.into_owned())
    });
    let board_id = if let Some(board_id) = query_board {
        board_id
    } else if headers.contains_key("x-rizzy-json-request") {
        let value: Value = serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?;
        required(&value, "board_id")?
    } else {
        url::form_urlencoded::parse(&body)
            .find(|(key, _)| key == "board_id")
            .map(|(_, value)| value.into_owned())
            .ok_or_else(|| bad("board_id is required"))?
    };
    create_card(State(app), Path((account, board_id)), headers, body).await
}
async fn create_or_resume_draft(
    app: &App,
    p: &Principal,
    account: &str,
    board_id: &str,
    headers: &HeaderMap,
    force_new: bool,
) -> Result<Response> {
    let _guard = app.write_lock.lock().await;
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let existing: Option<i64> = if force_new {
        None
    } else {
        sqlx::query_scalar(
            "SELECT number FROM cards WHERE board_id=? AND creator_id=? AND status='drafted' ORDER BY created_at,id LIMIT 1",
        )
        .bind(board_id)
        .bind(&p.user_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?
    };
    let timestamp = now();
    let number = if let Some(number) = existing {
        sqlx::query("UPDATE cards SET created_at=?,updated_at=?,last_active_at=? WHERE account_id=? AND number=?")
            .bind(&timestamp).bind(&timestamp).bind(&timestamp).bind(&p.account_id).bind(number)
            .execute(&mut *tx).await.map_err(db_err)?;
        number
    } else {
        let number: i64 = sqlx::query_scalar(
            "UPDATE accounts SET cards_count=cards_count+1 WHERE id=? RETURNING cards_count",
        )
        .bind(&p.account_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        let card_id = id();
        sqlx::query("INSERT INTO cards(id,account_id,board_id,creator_id,number,title,description,status,created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,'','','drafted',?,?,?)")
            .bind(&card_id).bind(&p.account_id).bind(board_id).bind(&p.user_id).bind(number)
            .bind(&timestamp).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
        sqlx::query("INSERT INTO watches(card_id,user_id,watching) VALUES(?,?,1)")
            .bind(card_id)
            .bind(&p.user_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        number
    };
    tx.commit().await.map_err(db_err)?;
    Ok((
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("{}/{account}/cards/{number}/draft", base(headers)),
        )],
    )
        .into_response())
}

async fn show_draft(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let c = card(&app, &p, &number).await?;
    if c.get::<String, _>("status") != "drafted" {
        return Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("{}/{account}/cards/{number}", base(&headers)),
            )],
        )
            .into_response());
    }
    if headers.contains_key("x-rizzy-json-request") {
        return Ok((
            StatusCode::NOT_ACCEPTABLE,
            Json(json!({"status":406,"error":"Not Acceptable"})),
        )
            .into_response());
    }
    let title = escape(&c.get::<String, _>("title"));
    let description = c.get::<String, _>("description");
    let board_id: String = c.get("board_id");
    let board_name = escape(&board(&app, &p, &board_id).await?.get::<String, _>("name"));
    let creator_name: String = sqlx::query_scalar("SELECT name FROM users WHERE id=?")
        .bind(c.get::<String, _>("creator_id"))
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let creator_name = escape(&creator_name);
    let html = format!(
        r##"<!doctype html>
<html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>New card | Rizzy</title>
<link rel="stylesheet" href="/assets/lexxy/lexxy.css"><link rel="stylesheet" href="/assets/lexxy/rizzy-dark.css"><script type="importmap">{{"imports":{{"@rails/activestorage":"/assets/lexxy/activestorage.esm.js"}}}}</script><script type="module" src="/assets/lexxy/lexxy.js"></script>
<style>
:root{{font:16px/1.5 system-ui,sans-serif;background:#1b1b1b;color:#f5f0e8}}
*{{box-sizing:border-box}}body{{margin:0}}a{{color:inherit}}button,textarea{{font:inherit}}
.skip{{position:absolute;left:-9999px}}.skip:focus{{left:16px;top:12px;z-index:10}}
header{{height:70px;display:flex;align-items:center;padding:0 max(20px,4vw);border-bottom:1px solid #383735}}
header a{{font-size:29px;font-weight:800;letter-spacing:-.055em;text-decoration:none}}
main{{width:min(900px,calc(100% - 32px));margin:36px auto 70px}}
nav a{{color:#d6c9b5;text-decoration:none;font-weight:650}}nav a:hover{{text-decoration:underline}}
.card-perma{{margin-top:24px;background:#292826;border:1px solid #46433d;border-radius:18px;box-shadow:0 14px 40px #0005;overflow:hidden}}
.card-head{{padding:18px 28px 15px;border-bottom:1px solid #45433e;color:#bbb5aa;font-size:14px}}
.card-head strong{{display:block;color:#f5f0e8;font-size:16px;margin-top:4px}}
.card-body{{padding:24px 28px 28px}}label{{display:block;color:#bdb7ac;font-size:14px;font-weight:600;margin:0 0 22px}}
textarea{{display:block;resize:vertical;width:100%;margin-top:8px;border:1px solid #57534d;background:#201f1e;color:#f8f4ed;border-radius:10px;padding:12px 14px;outline:none}}
textarea:focus{{border-color:#e6b857;box-shadow:0 0 0 3px #e6b85729}}
#title{{min-height:58px;font-size:25px;font-weight:750;line-height:1.3}}
lexxy-editor{{display:block;min-height:180px;margin-top:8px;border:1px solid #57534d;background:#201f1e;color:#f8f4ed;border-radius:10px;padding:12px 14px}}
lexxy-editor:focus-within{{border-color:#e6b857;box-shadow:0 0 0 3px #e6b85729}}
.card-foot{{padding:18px 28px;background:#242320;border-top:1px solid #45433e;display:flex;align-items:center;justify-content:space-between;gap:16px;flex-wrap:wrap}}
.byline{{color:#afa89d;font-size:14px}}.actions{{display:flex;gap:10px;align-items:center;flex-wrap:wrap}}
button{{border:1px solid #d5a94c;background:#e7bd61;color:#201b12;border-radius:9px;padding:9px 15px;font-weight:700;cursor:pointer}}
button:hover{{background:#f0ca77}}button.secondary{{border-color:#666158;background:#393733;color:#f6f0e8}}button.secondary:hover{{background:#4a463f}}
form{{display:inline}}@media(max-width:600px){{main{{margin-top:22px}}.card-body,.card-head,.card-foot{{padding-left:18px;padding-right:18px}}}}
</style></head>
<body><a class="skip" href="#main">Skip to main content</a><header><a href="/">Rizzy</a></header>
<main id="main"><nav><a href="/{account}/boards/{board_id}">← Back to {board_name}</a></nav>
<article class="card-perma card-perma--draft"><div class="card-head">Card number {number}<strong>{board_name}</strong></div>
<div class="card-body"><label>Name it<textarea id="title" maxlength="255" placeholder="Name it…">{title}</textarea></label>
<label for="description">Add some notes, context, pictures, or video</label>
<lexxy-editor id="description" name="card[description]" value="{description_value}" placeholder="Add some notes, context, pictures, or video about this…" data-direct-upload-url="/{account}/rails/active_storage/direct_uploads" data-blob-url-template="/{account}/rails/active_storage/blobs/redirect/:signed_id/:filename">
<lexxy-prompt trigger="@" src="/{account}/prompts/boards/{board_id}/users" name="mention"></lexxy-prompt>
<lexxy-prompt trigger="#" src="/{account}/prompts/cards" name="card" insert-editable-text="true" remote-filtering="true" supports-space-in-searches="true"></lexxy-prompt>
</lexxy-editor></div>
<div class="card-foot"><span class="byline">Drafted · By {creator_name}</span><div class="actions">
<button id="save" type="button" class="secondary">Save draft</button>
<form action="/{account}/cards/{number}/publish" method="post"><button type="submit">Create card</button></form>
<form action="/{account}/cards/{number}/publish" method="post"><input type="hidden" name="creation_type" value="add_another"><button type="submit" class="secondary">Create and add another</button></form>
</div></div></article></main>
<script>
const editor = document.getElementById('description');
let saveTimer;
let saving = Promise.resolve(true);
function saveDraft() {{
  clearTimeout(saveTimer);
  const body = JSON.stringify({{card:{{title:document.getElementById('title').value,description:editor.value}}}});
  saving = saving.catch(() => false).then(async () => {{
    try {{
      const response = await fetch('/{account}/cards/{number}', {{method:'PATCH',headers:{{'Accept':'application/json','Content-Type':'application/json'}},body}});
      if (!response.ok) throw new Error('Could not save draft');
      return true;
    }} catch (error) {{ alert(error.message); return false; }}
  }});
  return saving;
}}
function scheduleSave() {{ clearTimeout(saveTimer); saveTimer = setTimeout(saveDraft, 3000); }}
document.getElementById('title').addEventListener('input', scheduleSave);
document.getElementById('title').addEventListener('blur', saveDraft);
editor.addEventListener('lexxy:change', scheduleSave);
editor.addEventListener('focusout', saveDraft);
document.getElementById('save').onclick = saveDraft;
document.querySelectorAll('form').forEach(form => form.addEventListener('submit', async event => {{
  event.preventDefault();
  if (await saveDraft()) form.submit();
}}));
</script></body></html>"##,
        description_value = escape(&description)
    );
    Ok(Html(html).into_response())
}

async fn publish_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let board_id: String = c.get("board_id");
    let card_id: String = c.get("id");
    let timestamp = now();
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("UPDATE cards SET status='published',title=CASE WHEN trim(title)='' THEN 'Untitled' ELSE title END,created_at=?,updated_at=? WHERE id=?")
        .bind(&timestamp).bind(&timestamp).bind(&card_id).execute(&mut *tx).await.map_err(db_err)?;
    let event_id = id();
    sqlx::query("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,particulars,created_at) VALUES(?,?,?,?,?,'Card',?,'card_published','{}',?)")
        .bind(&event_id).bind(&p.account_id).bind(&board_id).bind(&card_id).bind(&p.user_id)
        .bind(&card_id).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
    queue_webhooks(
        &mut tx,
        &p.account_id,
        &board_id,
        &event_id,
        "card_published",
        &timestamp,
    )
    .await?;
    queue_notifications(
        &mut tx,
        &p,
        &board_id,
        &card_id,
        &event_id,
        "Card",
        "card_published",
        &json!({}),
        &timestamp,
    )
    .await?;
    create_new_mentions(
        &mut tx,
        &p,
        &board_id,
        &card_id,
        "Card",
        &card_id,
        &c.get::<String, _>("description"),
    )
    .await?;
    tx.commit().await.map_err(db_err)?;
    if headers.contains_key("x-rizzy-json-request") {
        return Ok(StatusCode::CREATED.into_response());
    }
    let add_another = url::form_urlencoded::parse(&body)
        .any(|(key, value)| key == "creation_type" && value == "add_another");
    if add_another {
        return create_new_draft(&app, &p, &account, &board_id, &headers).await;
    }
    Ok((
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("{}/{account}/boards/{board_id}", base(&headers)),
        )],
    )
        .into_response())
}

async fn create_new_draft(
    app: &App,
    p: &Principal,
    account: &str,
    board_id: &str,
    headers: &HeaderMap,
) -> Result<Response> {
    // Publishing with "add another" creates a fresh draft even if one remains.
    create_or_resume_draft(app, p, account, board_id, headers, true).await
}
async fn update_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let v = nested(&body, "card");
    let mut title = string(v, "title").unwrap_or_else(|| c.get("title"));
    let old_title: String = c.get("title");
    let status: String = c.get("status");
    if status == "published" && title.trim().is_empty() {
        title = "Untitled".into();
    }
    let description = string(v, "description").unwrap_or_else(|| c.get("description"));
    let (plain_description, _, _) =
        storage::render_rich_text(&app, &p.account_id, p.slug, &description).await?;
    let search_text = search_content(plain_description);
    let created_at = timestamp_param(v, "created_at")?.unwrap_or_else(|| c.get("created_at"));
    let custom_last_active_at = timestamp_param(v, "last_active_at")?;
    let last_active_at = custom_last_active_at.clone().unwrap_or_else(now);
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query(
        "UPDATE cards SET title=?,description=?,search_text=?,status=?,created_at=?,updated_at=?,last_active_at=? WHERE id=?",
    )
    .bind(&title)
    .bind(&description)
    .bind(&search_text)
    .bind(&status)
    .bind(&created_at)
    .bind(now())
    .bind(&last_active_at)
    .bind(c.get::<String, _>("id"))
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
    let replaced_image_keys =
        storage::attach_card_image_tx(&mut tx, &p, &c.get::<String, _>("id"), v.get("image"))
            .await?;
    let removed_embed_keys =
        storage::sync_embeds_tx(&mut tx, &p, "Card", &c.get::<String, _>("id"), &description)
            .await?;
    if status == "published" {
        create_new_mentions(
            &mut tx,
            &p,
            &c.get::<String, _>("board_id"),
            &c.get::<String, _>("id"),
            "Card",
            &c.get::<String, _>("id"),
            &description,
        )
        .await?;
    }
    tx.commit().await.map_err(db_err)?;
    storage::remove_files(replaced_image_keys).await;
    storage::remove_files(removed_embed_keys).await;
    if status == "published" && title != old_title && !old_title.is_empty() {
        record_event(
            &app,
            &p,
            &c.get::<String, _>("board_id"),
            &c.get::<String, _>("id"),
            "Card",
            &c.get::<String, _>("id"),
            "card_title_changed",
            json!({"old_title":&old_title,"new_title":&title}),
        )
        .await?;
        let who = escape(&actor_name(&app, &p).await?);
        system_comment(
            &app,
            &p,
            &c.get::<String, _>("id"),
            &format!(
                "{who} <strong>changed the title</strong> from “{}” to “{}”.",
                escape(&old_title),
                escape(&title)
            ),
        )
        .await?;
    }
    if let Some(last_active_at) = custom_last_active_at {
        sqlx::query("UPDATE cards SET last_active_at=? WHERE id=?")
            .bind(last_active_at)
            .bind(c.get::<String, _>("id"))
            .execute(&app.db)
            .await
            .map_err(db_err)?;
    }
    let c = card(&app, &p, &number).await?;
    Ok(Json(card_json(&app, &p, &c, &base(&headers), true).await?))
}
async fn delete_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let b = board(&app, &p, &c.get::<String, _>("board_id")).await?;
    p.admin(&b.get::<String, _>("creator_id")).or_else(|_| {
        if p.user_id == c.get::<String, _>("creator_id") {
            Ok(())
        } else {
            Err(err(StatusCode::FORBIDDEN, "Card administrator required"))
        }
    })?;
    let card_id: String = c.get("id");
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let keys = storage::purge_card_storage_tx(&mut tx, &p.account_id, Some(&card_id), None).await?;
    sqlx::query("DELETE FROM cards WHERE id=?")
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    storage::remove_files(keys).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn set_flag(
    app: &App,
    p: &Principal,
    number: &str,
    flag: &str,
    value: i64,
) -> Result<StatusCode> {
    p.write()?;
    let c = card(app, p, number).await?;
    let sql = match (flag, value) {
        ("closed", 1) => {
            "UPDATE cards SET closed=?,postponed=0,updated_at=?,last_active_at=? WHERE id=?"
        }
        ("closed", 0) => "UPDATE cards SET closed=?,updated_at=?,last_active_at=? WHERE id=?",
        ("postponed", 1) => {
            "UPDATE cards SET postponed=?,closed=0,column_id=NULL,updated_at=?,last_active_at=? WHERE id=?"
        }
        ("postponed", 0) => "UPDATE cards SET postponed=?,updated_at=?,last_active_at=? WHERE id=?",
        ("golden", _) => "UPDATE cards SET golden=?,updated_at=?,last_active_at=? WHERE id=?",
        _ => return Err(bad("invalid flag")),
    };
    let timestamp = now();
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query(sql)
        .bind(value)
        .bind(&timestamp)
        .bind(&timestamp)
        .bind(c.get::<String, _>("id"))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    if flag == "closed" && value == 1 {
        sqlx::query("INSERT OR IGNORE INTO closures(card_id,user_id,created_at) VALUES(?,?,?)")
            .bind(c.get::<String, _>("id"))
            .bind(&p.user_id)
            .bind(&timestamp)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    } else if (flag == "closed" && value == 0) || (flag == "postponed" && value == 1) {
        sqlx::query("DELETE FROM closures WHERE card_id=?")
            .bind(c.get::<String, _>("id"))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    if flag == "postponed" && value == 1 {
        sqlx::query("DELETE FROM card_activity_spikes WHERE card_id=?")
            .bind(c.get::<String, _>("id"))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    let card_id: String = c.get("id");
    if flag == "closed" && (c.get::<i64, _>("closed") != value) {
        let action = if value == 1 {
            "card_closed"
        } else {
            "card_reopened"
        };
        record_event(
            app,
            p,
            &c.get::<String, _>("board_id"),
            &card_id,
            "Card",
            &card_id,
            action,
            json!({}),
        )
        .await?;
        let who = escape(&actor_name(app, p).await?);
        let body = if value == 1 {
            format!("<strong>Moved</strong> to “Done” by {who}")
        } else {
            format!("<strong>Reopened</strong> by {who}")
        };
        system_comment(app, p, &card_id, &body).await?;
        if value == 0 {
            record_activity_spike(app, &p.account_id, &card_id).await?;
        }
    } else if flag == "postponed" && value == 1 {
        record_event(
            app,
            p,
            &c.get::<String, _>("board_id"),
            &card_id,
            "Card",
            &card_id,
            "card_postponed",
            json!({}),
        )
        .await?;
        let who = escape(&actor_name(app, p).await?);
        system_comment(
            app,
            p,
            &card_id,
            &format!("{who} <strong>moved</strong> this to “Not Now”"),
        )
        .await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn set_card_reading(
    app: &App,
    p: &Principal,
    number: &str,
    read: bool,
) -> Result<StatusCode> {
    p.write()?;
    let card = card(app, p, number).await?;
    let card_id: String = card.get("id");
    let board_id: String = card.get("board_id");
    let timestamp = now();
    let cutoff = (Utc::now() - chrono::Duration::minutes(5))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let mut tx = app.db.begin().await.map_err(db_err)?;
    if read {
        sqlx::query("UPDATE notifications SET read_at=?,unread_count=0,updated_at=? WHERE card_id=? AND user_id=? AND account_id=?")
            .bind(&timestamp).bind(&timestamp).bind(&card_id).bind(&p.user_id).bind(&p.account_id)
            .execute(&mut *tx).await.map_err(db_err)?;
    } else {
        sqlx::query("UPDATE notifications SET read_at=NULL,unread_count=1,updated_at=? WHERE card_id=? AND user_id=? AND account_id=?")
            .bind(&timestamp).bind(&card_id).bind(&p.user_id).bind(&p.account_id)
            .execute(&mut *tx).await.map_err(db_err)?;
    }
    let touched=sqlx::query("UPDATE accesses SET accessed_at=? WHERE board_id=? AND user_id=? AND (accessed_at IS NULL OR accessed_at<?)")
        .bind(&timestamp).bind(&board_id).bind(&p.user_id).bind(cutoff)
        .execute(&mut *tx).await.map_err(db_err)?;
    if touched.rows_affected() > 0 {
        sqlx::query("UPDATE boards SET updated_at=? WHERE id=?")
            .bind(&timestamp)
            .bind(&board_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(if read {
        StatusCode::CREATED
    } else {
        StatusCode::NO_CONTENT
    })
}
async fn read_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    set_card_reading(&app, &p, &number, true).await
}
async fn unread_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    set_card_reading(&app, &p, &number, false).await
}
async fn close_card(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &h, &a).await?;
    set_flag(&app, &p, &n, "closed", 1).await
}
async fn open_card(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &h, &a).await?;
    set_flag(&app, &p, &n, "closed", 0).await
}
async fn postpone_card(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &h, &a).await?;
    set_flag(&app, &p, &n, "postponed", 1).await
}
async fn unpostpone_card(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &h, &a).await?;
    set_flag(&app, &p, &n, "postponed", 0).await
}
async fn drop_card_to(
    app: &App,
    account: &str,
    number: &str,
    headers: &HeaderMap,
    destination: &str,
    column_id: Option<&str>,
) -> Result<Response> {
    let p = principal(app, headers, account).await?;
    p.write()?;
    let c = card(app, &p, number).await?;
    let board_id: String = c.get("board_id");
    match destination {
        "closure" => {
            set_flag(app, &p, number, "closed", 1).await?;
        }
        "not_now" => {
            set_flag(app, &p, number, "postponed", 1).await?;
        }
        "stream" => {
            send_card_to_stream(app, &p, number).await?;
        }
        "column" => {
            let id = column_id.ok_or_else(|| bad("column_id is required"))?;
            triage_card_into(app, &p, number, id).await?;
        }
        _ => return Err(bad("invalid destination")),
    }
    board_page::drop_column_response(app, &p, &board_id, destination, column_id).await
}
async fn drop_card_closure(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    drop_card_to(&app, &account, &number, &headers, "closure", None).await
}
async fn drop_card_not_now(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    drop_card_to(&app, &account, &number, &headers, "not_now", None).await
}
async fn drop_card_stream(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    drop_card_to(&app, &account, &number, &headers, "stream", None).await
}
async fn drop_card_column(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    Query(query): Query<HashMap<String, String>>,
    headers: HeaderMap,
) -> Result<Response> {
    drop_card_to(
        &app,
        &account,
        &number,
        &headers,
        "column",
        query.get("column_id").map(String::as_str),
    )
    .await
}
async fn gild_card(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &h, &a).await?;
    set_flag(&app, &p, &n, "golden", 1).await
}
async fn ungild_card(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &h, &a).await?;
    set_flag(&app, &p, &n, "golden", 0).await
}

async fn triage_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    let column = required(&body, "column_id")?;
    triage_card_into(&app, &p, &number, &column).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn triage_card_into(app: &App, p: &Principal, number: &str, column: &str) -> Result<()> {
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let col = checked_column(&app, &p, &c.get::<String, _>("board_id"), &column).await?;
    let timestamp = now();
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("UPDATE cards SET column_id=?,closed=0,postponed=0,updated_at=?,last_active_at=? WHERE id=?")
        .bind(column).bind(&timestamp).bind(&timestamp).bind(c.get::<String,_>("id")).execute(&mut *tx).await.map_err(db_err)?;
    for table in ["closures", "card_activity_spikes"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE card_id=?"))
            .bind(c.get::<String, _>("id"))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    record_event(
        &app,
        &p,
        &c.get::<String, _>("board_id"),
        &c.get::<String, _>("id"),
        "Card",
        &c.get::<String, _>("id"),
        "card_triaged",
        json!({"column":col.get::<String,_>("name")}),
    )
    .await?;
    let who = escape(&actor_name(&app, &p).await?);
    system_comment(
        &app,
        &p,
        &c.get::<String, _>("id"),
        &format!(
            "{who} <strong>moved</strong> this to “{}”",
            escape(&col.get::<String, _>("name"))
        ),
    )
    .await?;
    Ok(())
}
async fn untriage_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    send_card_to_stream(&app, &p, &number).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn send_card_to_stream(app: &App, p: &Principal, number: &str) -> Result<()> {
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let timestamp = now();
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("UPDATE cards SET column_id=NULL,closed=0,postponed=0,updated_at=?,last_active_at=? WHERE id=?")
        .bind(&timestamp).bind(&timestamp).bind(c.get::<String,_>("id")).execute(&mut *tx).await.map_err(db_err)?;
    for table in ["closures", "card_activity_spikes"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE card_id=?"))
            .bind(c.get::<String, _>("id"))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    record_event(
        &app,
        &p,
        &c.get::<String, _>("board_id"),
        &c.get::<String, _>("id"),
        "Card",
        &c.get::<String, _>("id"),
        "card_sent_back_to_triage",
        json!({}),
    )
    .await?;
    let who = escape(&actor_name(&app, &p).await?);
    system_comment(
        &app,
        &p,
        &c.get::<String, _>("id"),
        &format!("{who} <strong>moved</strong> this back to “Maybe?”"),
    )
    .await?;
    Ok(())
}
async fn move_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    method: Method,
    raw_body: Bytes,
) -> Result<Response> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.starts_with("application/x-www-form-urlencoded"));
    if method == Method::POST
        && (!form
            || !url::form_urlencoded::parse(&raw_body)
                .any(|(key, value)| key == "_method" && value == "patch"))
    {
        return Err(missing());
    }
    let body: Value = if form || raw_body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&raw_body).map_err(|_| bad("Invalid JSON"))?
    };
    let query_board = raw_query.as_deref().and_then(|raw| {
        url::form_urlencoded::parse(raw.as_bytes())
            .find(|(key, _)| key == "board_id")
            .map(|(_, value)| value.into_owned())
    });
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let destination = query_board
        .or_else(|| string(&body, "board_id"))
        .ok_or_else(|| bad("board_id is required"))?;
    let old_board = board(&app, &p, &c.get::<String, _>("board_id")).await?;
    let new_board = board(&app, &p, &destination).await?;
    if c.get::<String, _>("board_id") != destination {
        let card_id = c.get::<String, _>("id");
        let mut tx = app.db.begin().await.map_err(db_err)?;
        sqlx::query("UPDATE cards SET board_id=?,column_id=NULL,updated_at=? WHERE id=?")
            .bind(&destination)
            .bind(now())
            .bind(&card_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query("UPDATE events SET board_id=? WHERE card_id=?")
            .bind(&destination)
            .bind(&card_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        if new_board.get::<i64, _>("all_access") == 0
            && p.admin(&new_board.get::<String, _>("creator_id")).is_ok()
        {
            sqlx::query("INSERT OR IGNORE INTO accesses(board_id,user_id,involvement) SELECT ?,a.assignee_id,'access_only' FROM assignments a JOIN users u ON u.id=a.assignee_id WHERE a.card_id=? AND u.active=1")
                .bind(&destination)
                .bind(&card_id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
        for table in ["pins", "watches", "notifications"] {
            let query = format!(
                "DELETE FROM {table} WHERE card_id=? AND user_id NOT IN (SELECT user_id FROM accesses WHERE board_id=?)"
            );
            sqlx::query(&query)
                .bind(&card_id)
                .bind(&destination)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
        sqlx::query("DELETE FROM assignments WHERE card_id=? AND assignee_id NOT IN (SELECT user_id FROM accesses WHERE board_id=?)")
            .bind(&card_id)
            .bind(&destination)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        tx.commit().await.map_err(db_err)?;
        record_event(&app,&p,&destination,&c.get::<String,_>("id"),"Card",&c.get::<String,_>("id"),"card_board_changed",json!({"old_board":old_board.get::<String,_>("name"),"new_board":new_board.get::<String,_>("name")})).await?;
        let who = escape(&actor_name(&app, &p).await?);
        system_comment(
            &app,
            &p,
            &c.get::<String, _>("id"),
            &format!(
                "{who} <strong>moved</strong> this from “{}” to “{}”.",
                escape(&old_board.get::<String, _>("name")),
                escape(&new_board.get::<String, _>("name"))
            ),
        )
        .await?;
    }
    let c = card(&app, &p, &number).await?;
    if form {
        return Ok((
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/cards/{number}"))],
        )
            .into_response());
    }
    Ok(Json(card_json(&app, &p, &c, &base(&headers), true).await?).into_response())
}
async fn toggle_tag(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    body: Bytes,
) -> Result<Response> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"))
        || !headers.contains_key("x-rizzy-json-request");
    let query_title = raw_query.as_deref().and_then(|raw| {
        url::form_urlencoded::parse(raw.as_bytes())
            .find(|(key, _)| key == "tag_title")
            .map(|(_, value)| value.into_owned())
    });
    let tag_title = if let Some(value) = query_title {
        value
    } else if form {
        url::form_urlencoded::parse(&body)
            .find(|(key, _)| key == "tag_title")
            .map(|(_, value)| value.into_owned())
            .ok_or_else(|| bad("tag_title is required"))?
    } else {
        let json: Value = serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?;
        required(&json, "tag_title")?
    };
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let title = tag_title
        .trim()
        .trim_start_matches('#')
        .trim()
        .to_lowercase();
    if title.is_empty() {
        return Err(bad("tag_title is required"));
    }
    let tag_id = id();
    sqlx::query("INSERT INTO tags(id,account_id,title,created_at) VALUES(?,?,?,?) ON CONFLICT(account_id,title) DO NOTHING")
        .bind(&tag_id).bind(&p.account_id).bind(&title).bind(now()).execute(&app.db).await.map_err(db_err)?;
    let tag_id: String = sqlx::query_scalar("SELECT id FROM tags WHERE account_id=? AND title=?")
        .bind(&p.account_id)
        .bind(&title)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let card_id: String = c.get("id");
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM taggings WHERE card_id=? AND tag_id=?")
            .bind(&card_id)
            .bind(&tag_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
    if exists == 0 {
        sqlx::query("INSERT INTO taggings(card_id,tag_id) VALUES(?,?)")
            .bind(card_id)
            .bind(tag_id)
            .execute(&app.db)
            .await
            .map_err(db_err)?;
    } else {
        sqlx::query("DELETE FROM taggings WHERE card_id=? AND tag_id=?")
            .bind(card_id)
            .bind(tag_id)
            .execute(&app.db)
            .await
            .map_err(db_err)?;
    }
    if form {
        return Ok((
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/cards/{number}"))],
        )
            .into_response());
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn list_steps(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    let c = card(&app, &p, &number).await?;
    let rows = sqlx::query("SELECT * FROM steps WHERE card_id=? ORDER BY created_at,id")
        .bind(c.get::<String, _>("id"))
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    Ok(Json(json!(rows.iter().map(|r|json!({"id":r.get::<String,_>("id"),"content":r.get::<String,_>("content"),"completed":r.get::<i64,_>("completed")!=0})).collect::<Vec<_>>())))
}
async fn step_row(app: &App, card_id: &str, step: &str) -> Result<SqliteRow> {
    sqlx::query("SELECT * FROM steps WHERE id=? AND card_id=?")
        .bind(step)
        .bind(card_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)
}
fn step_value(row: &SqliteRow) -> Value {
    json!({"id":row.get::<String,_>("id"),"content":row.get::<String,_>("content"),"completed":row.get::<i64,_>("completed")!=0})
}
async fn show_step(
    State(app): Shared,
    Path((account, number, step)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let c = card(&app, &p, &number).await?;
    let row = step_row(&app, &c.get::<String, _>("id"), &step).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Ok(Json(step_value(&row)).into_response());
    }
    let step_id = escape(&row.get::<String, _>("id"));
    let content = escape(&row.get::<String, _>("content"));
    let checked = if row.get::<i64, _>("completed") != 0 {
        " checked"
    } else {
        ""
    };
    let path = format!("/{account}/cards/{number}/steps/{step_id}");
    let html = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Step | Rizzy</title></head><body><main><turbo-frame id="step_{step_id}"><li class="step"><form class="step" id="step-form"><input type="checkbox" id="step-completed" name="step[completed]" class="checkbox"{checked}><a class="step__content" href="{path}/edit">{content}</a></form></li></turbo-frame></main><script>document.getElementById('step-completed').addEventListener('change',async event => {{const response=await fetch('{path}',{{method:'PATCH',headers:{{'Accept':'application/json','Content-Type':'application/json'}},body:JSON.stringify({{step:{{completed:event.target.checked}}}})}});if(!response.ok) event.target.checked=!event.target.checked;}});</script></body></html>"#
    );
    Ok(Html(html).into_response())
}
async fn edit_step(
    State(app): Shared,
    Path((account, number, step)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let c = card(&app, &p, &number).await?;
    let row = step_row(&app, &c.get::<String, _>("id"), &step).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Ok((
            StatusCode::NOT_ACCEPTABLE,
            Json(json!({"status":406,"error":"Not Acceptable"})),
        )
            .into_response());
    }
    let step_id = escape(&row.get::<String, _>("id"));
    let content = escape(&row.get::<String, _>("content"));
    let checked = if row.get::<i64, _>("completed") != 0 {
        " checked"
    } else {
        ""
    };
    let path = format!("/{account}/cards/{number}/steps/{step_id}");
    let html = format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>Edit step | Rizzy</title></head><body><main><turbo-frame id="step_{step_id}"><div class="flex align-center gap-half"><form class="step" id="step-edit"><input type="checkbox" class="checkbox" disabled{checked}><input type="text" name="step[content]" class="input step__content step__content--edit hide-focus-ring" placeholder="Name this step…" required autofocus autocomplete="off" value="{content}"><button type="submit" class="btn btn--positive txt-xx-small">Save changes</button><a href="{path}">Cancel changes</a></form><button type="button" id="step-delete" class="btn btn--negative txt-xx-small">Delete this step</button></div></turbo-frame></main><script>document.getElementById('step-edit').addEventListener('submit',async event => {{event.preventDefault();const content=event.target.elements['step[content]'].value;const response=await fetch('{path}',{{method:'PATCH',headers:{{'Accept':'application/json','Content-Type':'application/json'}},body:JSON.stringify({{step:{{content}}}})}});if(response.ok) location.href='{path}';}});document.getElementById('step-delete').addEventListener('click',async () => {{const response=await fetch('{path}',{{method:'DELETE',headers:{{'Accept':'application/json'}}}});if(response.ok) location.href='/{account}/cards/{number}';}});</script></body></html>"#
    );
    Ok(Html(html).into_response())
}
async fn create_step(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let v = nested(&body, "step");
    let content = required(v, "content")?;
    let id = id();
    sqlx::query("INSERT INTO steps(id,card_id,content,created_at) VALUES(?,?,?,?)")
        .bind(&id)
        .bind(c.get::<String, _>("id"))
        .bind(&content)
        .bind(now())
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(created(
        format!("/{account}/cards/{number}/steps/{id}.json"),
        json!({"id":id,"content":content,"completed":false}),
    ))
}
async fn update_step(
    State(app): Shared,
    Path((account, number, step)): Path<(String, String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let old = sqlx::query("SELECT * FROM steps WHERE id=? AND card_id=?")
        .bind(&step)
        .bind(c.get::<String, _>("id"))
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)?;
    let v = nested(&body, "step");
    let content = string(v, "content").unwrap_or_else(|| old.get("content"));
    let completed = v
        .get("completed")
        .and_then(Value::as_bool)
        .unwrap_or(old.get::<i64, _>("completed") != 0);
    sqlx::query("UPDATE steps SET content=?,completed=? WHERE id=?")
        .bind(&content)
        .bind(completed as i64)
        .bind(&step)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(Json(
        json!({"id":step,"content":content,"completed":completed}),
    ))
}
async fn delete_step(
    State(app): Shared,
    Path((account, number, step)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let done = sqlx::query("DELETE FROM steps WHERE id=? AND card_id=?")
        .bind(step)
        .bind(c.get::<String, _>("id"))
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    if done.rows_affected() == 0 {
        return Err(missing());
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn comment_row(app: &App, card: &SqliteRow, id: &str) -> Result<SqliteRow> {
    sqlx::query("SELECT * FROM comments WHERE id=? AND card_id=?")
        .bind(id.trim_end_matches(".json"))
        .bind(card.get::<String, _>("id"))
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)
}
async fn comment_json(
    app: &App,
    p: &Principal,
    card: &SqliteRow,
    r: &SqliteRow,
    host: &str,
) -> Result<Value> {
    let id: String = r.get("id");
    let card_id: String = card.get("id");
    let number: i64 = card.get("number");
    let body: String = r.get("body");
    let created: String = r.get("created_at");
    let updated = r
        .get::<Option<String>, _>("updated_at")
        .unwrap_or_else(|| created.clone());
    let creator = user_json(app, p, &r.get::<String, _>("creator_id"), host).await?;
    let (plain, html) = if creator["role"] == "system" {
        (system_plain_text(&body), wrap_html(&body))
    } else {
        let (plain, html, _) = storage::render_rich_text(app, &p.account_id, p.slug, &body).await?;
        (plain, html)
    };
    Ok(
        json!({"id":id,"created_at":created,"updated_at":updated,"body":{"plain_text":plain,"html":html},"creator":creator,"card":{"id":card_id,"url":format!("{host}/{}/cards/{number}",p.slug)},"reactions_url":format!("{host}/{}/cards/{number}/comments/{id}/reactions",p.slug),"url":format!("{host}/{}/cards/{number}/comments/{id}",p.slug)}),
    )
}
async fn list_comments(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let c = card(&app, &p, &number).await?;
    let card_id: String = c.get("id");
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE card_id=?")
        .bind(&card_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let rows = sqlx::query(
        "SELECT * FROM comments WHERE card_id=? ORDER BY created_at,id LIMIT ? OFFSET ?",
    )
    .bind(&card_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let host = base(&headers);
    let mut out = Vec::new();
    for r in &rows {
        out.push(comment_json(&app, &p, &c, r, &host).await?);
    }
    Ok(paginated(json!(out), total, page, &uri, &headers))
}
async fn create_comment(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.starts_with("application/x-www-form-urlencoded"));
    let body: Value = if form {
        let fields: HashMap<String, String> = url::form_urlencoded::parse(&body)
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        let mut comment = json!({"body":fields.get("comment[body]")});
        if let Some(created_at) = fields.get("comment[created_at]") {
            comment["created_at"] = json!(created_at);
        }
        json!({"comment":comment})
    } else {
        serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?
    };
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    if c.get::<String, _>("status") != "published" {
        return Err(err(StatusCode::FORBIDDEN, "Card is not commentable"));
    }
    let comment_input = nested(&body, "comment");
    let content = required(comment_input, "body")?;
    let (plain_body, _, _) =
        storage::render_rich_text(&app, &p.account_id, p.slug, &content).await?;
    let search_text = search_content(plain_body);
    let id = id();
    let timestamp = now();
    let created_at =
        timestamp_param(comment_input, "created_at")?.unwrap_or_else(|| timestamp.clone());
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("INSERT INTO comments(id,card_id,creator_id,body,search_text,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
        .bind(&id)
        .bind(c.get::<String, _>("id"))
        .bind(&p.user_id)
        .bind(&content)
        .bind(&search_text)
        .bind(&created_at)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    storage::sync_embeds_tx(&mut tx, &p, "Comment", &id, &content).await?;
    create_new_mentions(
        &mut tx,
        &p,
        &c.get::<String, _>("board_id"),
        &c.get::<String, _>("id"),
        "Comment",
        &id,
        &content,
    )
    .await?;
    tx.commit().await.map_err(db_err)?;
    record_event(
        &app,
        &p,
        &c.get::<String, _>("board_id"),
        &c.get::<String, _>("id"),
        "Comment",
        &id,
        "comment_created",
        json!({}),
    )
    .await?;
    let period: i64 =
        sqlx::query_scalar("SELECT auto_postpone_period_in_days FROM boards WHERE id=?")
            .bind(c.get::<String, _>("board_id"))
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
    let cutoff = (Utc::now() - chrono::Duration::seconds(period * 24 * 60 * 60 * 33 / 100))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let counts = sqlx::query("SELECT COUNT(*) AS comments,COUNT(DISTINCT creator_id) AS people FROM comments WHERE card_id=? AND created_at>=?")
        .bind(c.get::<String, _>("id")).bind(cutoff)
        .fetch_one(&app.db).await.map_err(db_err)?;
    if counts.get::<i64, _>("comments") >= 3 && counts.get::<i64, _>("people") >= 2 {
        record_activity_spike(&app, &p.account_id, &c.get::<String, _>("id")).await?;
    }
    let r = comment_row(&app, &c, &id).await?;
    if wants_turbo_stream(&headers) {
        let target = format!("new_comment_card_{}", escape(&c.get::<String, _>("id")));
        let comment = comment_edit_page::rendered_comment_frame(&app, &p, &c, &r).await?;
        let fresh_form = card_page::new_comment_form(&p, &c);
        return Ok((
            [(header::CONTENT_TYPE, "text/vnd.turbo-stream.html; charset=utf-8")],
            format!("<turbo-stream action=\"before\" target=\"{target}\"><template>{comment}</template></turbo-stream><turbo-stream action=\"update\" target=\"{target}\"><template>{fresh_form}</template></turbo-stream>"),
        ).into_response());
    }
    if headers.contains_key("x-rizzy-json-request") {
        Ok(created(
            format!("/{account}/cards/{number}/comments/{id}.json"),
            comment_json(&app, &p, &c, &r, &base(&headers)).await?,
        ))
    } else {
        Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"))
    }
}
async fn show_comment(
    State(app): Shared,
    Path((account, number, id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let c = card(&app, &p, &number).await?;
    let r = comment_row(&app, &c, &id).await?;
    if !headers.contains_key("x-rizzy-json-request") {
        return Ok(
            Html(comment_edit_page::rendered_comment_frame(&app, &p, &c, &r).await?)
                .into_response(),
        );
    }
    Ok(Json(comment_json(&app, &p, &c, &r, &base(&headers)).await?).into_response())
}
fn wants_turbo_stream(headers: &HeaderMap) -> bool {
    headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/vnd.turbo-stream.html"))
}
async fn comment_replace_stream(
    app: &App,
    account: &str,
    number: &str,
    id: &str,
    headers: &HeaderMap,
) -> Result<Response> {
    let p = principal(app, headers, account).await?;
    let c = card(app, &p, number).await?;
    let r = comment_row(app, &c, id).await?;
    let frame = comment_edit_page::rendered_comment_frame(app, &p, &c, &r).await?;
    let stream = format!(
        "<turbo-stream action=\"replace\" target=\"container_comment_{}\"><template>{frame}</template></turbo-stream>",
        escape(id)
    );
    Ok((
        [(
            header::CONTENT_TYPE,
            "text/vnd.turbo-stream.html; charset=utf-8",
        )],
        stream,
    )
        .into_response())
}
async fn update_comment(
    State(app): Shared,
    Path((account, number, id)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.starts_with("application/x-www-form-urlencoded"));
    let body: Value = if form {
        let content = url::form_urlencoded::parse(&body)
            .find(|(key, _)| key == "comment[body]")
            .map(|(_, value)| value.into_owned())
            .ok_or_else(|| bad("body is required"))?;
        json!({"comment":{"body":content}})
    } else {
        serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?
    };
    if !wants_turbo_stream(&headers) && !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let result = update_comment_data(&app, &account, &number, &id, &headers, &body).await?;
    if wants_turbo_stream(&headers) {
        comment_replace_stream(&app, &account, &number, &id, &headers).await
    } else {
        Ok(Json(result).into_response())
    }
}

async fn update_or_delete_comment_form(
    State(app): Shared,
    Path((account, number, id)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|content_type| content_type.starts_with("application/x-www-form-urlencoded"));
    if !form {
        return Err(missing());
    }
    let fields: HashMap<String, String> = url::form_urlencoded::parse(&body)
        .map(|(key, value)| (key.into_owned(), value.into_owned()))
        .collect();
    match fields.get("_method").map(String::as_str) {
        Some("patch") => {
            let content = fields
                .get("comment[body]")
                .ok_or_else(|| bad("body is required"))?;
            update_comment_data(
                &app,
                &account,
                &number,
                &id,
                &headers,
                &json!({"comment":{"body":content}}),
            )
            .await?;
            if wants_turbo_stream(&headers) {
                return comment_replace_stream(&app, &account, &number, &id, &headers).await;
            }
        }
        Some("delete") => {
            let mut delete_headers = headers;
            delete_headers.insert("x-rizzy-json-request", HeaderValue::from_static("1"));
            let response = delete_comment(
                State(app),
                Path((account.clone(), number.clone(), id)),
                delete_headers,
            )
            .await?;
            if response.status() == StatusCode::OK {
                return Ok(response);
            }
        }
        _ => return Err(missing()),
    }
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, format!("/{account}/cards/{number}"))],
    )
        .into_response())
}

async fn update_comment_data(
    app: &App,
    account: &str,
    number: &str,
    id: &str,
    headers: &HeaderMap,
    body: &Value,
) -> Result<Value> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let r = comment_row(&app, &c, &id).await?;
    if r.get::<String, _>("creator_id") != p.user_id {
        return Err(err(StatusCode::FORBIDDEN, "Comment creator required"));
    }
    let content = required(nested(&body, "comment"), "body")?;
    let (plain_body, _, _) =
        storage::render_rich_text(&app, &p.account_id, p.slug, &content).await?;
    let search_text = search_content(plain_body);
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("UPDATE comments SET body=?,search_text=?,updated_at=? WHERE id=?")
        .bind(&content)
        .bind(&search_text)
        .bind(now())
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    let keys = storage::sync_embeds_tx(&mut tx, &p, "Comment", &id, &content).await?;
    create_new_mentions(
        &mut tx,
        &p,
        &c.get::<String, _>("board_id"),
        &c.get::<String, _>("id"),
        "Comment",
        &id,
        &content,
    )
    .await?;
    tx.commit().await.map_err(db_err)?;
    storage::remove_files(keys).await;
    let r = comment_row(&app, &c, &id).await?;
    comment_json(&app, &p, &c, &r, &base(&headers)).await
}
async fn delete_comment(
    State(app): Shared,
    Path((account, number, comment)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let r = comment_row(&app, &c, &comment).await?;
    if r.get::<String, _>("creator_id") != p.user_id {
        return Err(err(StatusCode::FORBIDDEN, "Comment creator required"));
    }
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let keys = storage::sync_embeds_tx(&mut tx, &p, "Comment", &comment, "").await?;
    sqlx::query(
        "DELETE FROM events WHERE eventable_type='Comment' AND eventable_id=? AND card_id=?",
    )
    .bind(&comment)
    .bind(c.get::<String, _>("id"))
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
    sqlx::query(
        "DELETE FROM reactions WHERE reactable_type='Comment' AND reactable_id=? AND account_id=?",
    )
    .bind(&comment)
    .bind(&p.account_id)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
    let done = sqlx::query("DELETE FROM comments WHERE id=? AND card_id=? AND creator_id=?")
        .bind(&comment)
        .bind(c.get::<String, _>("id"))
        .bind(&p.user_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    if done.rows_affected() == 0 {
        return Err(missing());
    }
    tx.commit().await.map_err(db_err)?;
    storage::remove_files(keys).await;
    if wants_turbo_stream(&headers) {
        Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            format!(
                "<turbo-stream action=\"remove\" target=\"container_comment_{}\"></turbo-stream>",
                escape(&comment)
            ),
        )
            .into_response())
    } else if headers.contains_key("x-rizzy-json-request") {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"))
    }
}

async fn toggle_assignment(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
    body: Bytes,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let query_assignee = raw_query.as_deref().and_then(|raw| {
        url::form_urlencoded::parse(raw.as_bytes())
            .find(|(key, _)| key == "assignee_id")
            .map(|(_, value)| value.into_owned())
    });
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"))
        || !headers.contains_key("x-rizzy-json-request");
    let assignee_id = if let Some(value) = query_assignee {
        value
    } else if form {
        url::form_urlencoded::parse(&body)
            .find(|(key, _)| key == "assignee_id")
            .map(|(_, value)| value.into_owned())
            .ok_or_else(|| bad("assignee_id is required"))?
    } else {
        let json: Value = serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?;
        required(&json, "assignee_id")?
    };
    toggle_assignment_to(&app, &p, &c, &assignee_id).await?;
    if form {
        return Ok((
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/cards/{number}"))],
        )
            .into_response());
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}

async fn toggle_self_assignment(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    toggle_assignment_to(&app, &p, &c, &p.user_id).await
}

async fn toggle_assignment_to(
    app: &App,
    p: &Principal,
    c: &SqliteRow,
    assignee_id: &str,
) -> Result<StatusCode> {
    let board_id: String = c.get("board_id");
    let card_id: String = c.get("id");
    let assignee=sqlx::query("SELECT u.id,u.name FROM users u JOIN boards b ON b.id=? WHERE u.id=? AND u.account_id=? AND u.active=1 AND u.role!='system' AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=u.id)")
        .bind(&board_id).bind(&assignee_id).bind(&p.account_id).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    let existed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM assignments WHERE card_id=? AND assignee_id=?")
            .bind(&card_id)
            .bind(&assignee_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
    let who = escape(&actor_name(&app, &p).await?);
    let target = escape(&assignee.get::<String, _>("name"));
    if existed > 0 {
        sqlx::query("DELETE FROM assignments WHERE card_id=? AND assignee_id=?")
            .bind(&card_id)
            .bind(&assignee_id)
            .execute(&app.db)
            .await
            .map_err(db_err)?;
        record_event(
            &app,
            &p,
            &board_id,
            &card_id,
            "Card",
            &card_id,
            "card_unassigned",
            json!({"assignee_ids":[&assignee_id]}),
        )
        .await?;
        system_comment(
            &app,
            &p,
            &card_id,
            &format!("{who} <strong>unassigned</strong> from {target}."),
        )
        .await?;
    } else {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assignments WHERE card_id=?")
            .bind(&card_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
        if count >= 100 {
            return Err(err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Card already has the maximum of 100 assignees",
            ));
        }
        sqlx::query("INSERT INTO assignments(id,account_id,card_id,assignee_id,assigner_id,created_at) VALUES(?,?,?,?,?,?)")
            .bind(id()).bind(&p.account_id).bind(&card_id).bind(&assignee_id).bind(&p.user_id).bind(now()).execute(&app.db).await.map_err(db_err)?;
        record_event(
            &app,
            &p,
            &board_id,
            &card_id,
            "Card",
            &card_id,
            "card_assigned",
            json!({"assignee_ids":[&assignee_id]}),
        )
        .await?;
        sqlx::query("INSERT INTO watches(card_id,user_id,watching) VALUES(?,?,1) ON CONFLICT(card_id,user_id) DO UPDATE SET watching=1")
            .bind(&card_id).bind(&assignee_id).execute(&app.db).await.map_err(db_err)?;
        system_comment(
            &app,
            &p,
            &card_id,
            &format!("{who} <strong>assigned</strong> this to {target}."),
        )
        .await?;
        record_activity_spike(&app, &p.account_id, &card_id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn set_watch(app: &App, p: &Principal, number: &str, watching: bool) -> Result<StatusCode> {
    p.write()?;
    let c = card(app, p, number).await?;
    sqlx::query("INSERT INTO watches(card_id,user_id,watching) VALUES(?,?,?) ON CONFLICT(card_id,user_id) DO UPDATE SET watching=excluded.watching")
        .bind(c.get::<String,_>("id")).bind(&p.user_id).bind(watching as i64).execute(&app.db).await.map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn set_pin(app: &App, p: &Principal, number: &str, pinned: bool) -> Result<StatusCode> {
    p.write()?;
    let c = card(app, p, number).await?;
    let card_id: String = c.get("id");
    if pinned {
        sqlx::query("INSERT OR IGNORE INTO pins(id,account_id,card_id,user_id,created_at) VALUES(?,?,?,?,?)")
        .bind(id()).bind(&p.account_id).bind(&card_id).bind(&p.user_id).bind(now()).execute(&app.db).await.map_err(db_err)?;
    } else {
        sqlx::query("DELETE FROM pins WHERE card_id=? AND user_id=?")
            .bind(&card_id)
            .bind(&p.user_id)
            .execute(&app.db)
            .await
            .map_err(db_err)?;
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn list_pins(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    let rows=sqlx::query("SELECT c.* FROM pins pin JOIN cards c ON c.id=pin.card_id JOIN boards b ON b.id=c.board_id WHERE pin.user_id=? AND pin.account_id=? AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?) ORDER BY pin.created_at DESC,pin.id DESC LIMIT 100")
        .bind(&p.user_id).bind(&p.account_id).bind(&p.user_id).fetch_all(&app.db).await.map_err(db_err)?;
    let host = base(&headers);
    let mut out = Vec::new();
    for r in &rows {
        out.push(card_json(&app, &p, r, &host, false).await?);
    }
    Ok(Json(json!(out)))
}

async fn reaction_json(
    app: &App,
    p: &Principal,
    r: &SqliteRow,
    host: &str,
    number: i64,
    comment: Option<&str>,
) -> Result<Value> {
    let id: String = r.get("id");
    let prefix = if let Some(comment) = comment {
        format!(
            "{host}/{}/cards/{number}/comments/{comment}/reactions",
            p.slug
        )
    } else {
        format!("{host}/{}/cards/{number}/reactions", p.slug)
    };
    Ok(
        json!({"id":id,"content":r.get::<String,_>("content"),"reacter":user_json(app,p,&r.get::<String,_>("reacter_id"),host).await?,"url":format!("{prefix}/{id}")}),
    )
}
async fn reactions_for(
    app: &App,
    p: &Principal,
    c: &SqliteRow,
    comment: Option<&str>,
    host: &str,
) -> Result<Value> {
    let (kind, target) = if let Some(id) = comment {
        let r = comment_row(app, c, id).await?;
        ("Comment", r.get::<String, _>("id"))
    } else {
        ("Card", c.get::<String, _>("id"))
    };
    let rows=sqlx::query("SELECT * FROM reactions WHERE account_id=? AND reactable_type=? AND reactable_id=? ORDER BY created_at,id")
        .bind(&p.account_id).bind(kind).bind(target).fetch_all(&app.db).await.map_err(db_err)?;
    let mut out = Vec::new();
    for r in &rows {
        out.push(reaction_json(app, p, r, host, c.get("number"), comment).await?);
    }
    Ok(json!(out))
}
async fn create_reaction(
    app: &App,
    p: &Principal,
    c: &SqliteRow,
    comment: Option<&str>,
    host: &str,
    body: Value,
) -> Result<Response> {
    p.write()?;
    let content = required(nested(&body, "reaction"), "content")?;
    if content.chars().count() > 16 {
        return Err(bad("content is too long"));
    }
    let (kind, target) = if let Some(id) = comment {
        let r = comment_row(app, c, id).await?;
        ("Comment", r.get::<String, _>("id"))
    } else {
        ("Card", c.get::<String, _>("id"))
    };
    let id = id();
    sqlx::query("INSERT INTO reactions(id,account_id,reactable_type,reactable_id,reacter_id,content,created_at) VALUES(?,?,?,?,?,?,?)")
        .bind(&id).bind(&p.account_id).bind(kind).bind(target).bind(&p.user_id).bind(content).bind(now()).execute(&app.db).await.map_err(db_err)?;
    let r = sqlx::query("SELECT * FROM reactions WHERE id=?")
        .bind(&id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    sqlx::query("UPDATE cards SET last_active_at=? WHERE id=?")
        .bind(now())
        .bind(c.get::<String, _>("id"))
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok((
        StatusCode::CREATED,
        Json(reaction_json(app, p, &r, host, c.get("number"), comment).await?),
    )
        .into_response())
}
async fn delete_reaction(
    app: &App,
    p: &Principal,
    c: &SqliteRow,
    comment: Option<&str>,
    reaction: &str,
) -> Result<StatusCode> {
    p.write()?;
    let (kind, target) = if let Some(id) = comment {
        let r = comment_row(app, c, id).await?;
        ("Comment", r.get::<String, _>("id"))
    } else {
        ("Card", c.get::<String, _>("id"))
    };
    let r=sqlx::query("SELECT reacter_id FROM reactions WHERE id=? AND account_id=? AND reactable_type=? AND reactable_id=?")
        .bind(reaction).bind(&p.account_id).bind(kind).bind(target).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    if r.get::<String, _>("reacter_id") != p.user_id {
        return Err(err(StatusCode::FORBIDDEN, "Reaction creator required"));
    }
    sqlx::query("DELETE FROM reactions WHERE id=?")
        .bind(reaction)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn list_card_reactions(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<Json<Value>> {
    let p = principal(&app, &h, &a).await?;
    let c = card(&app, &p, &n).await?;
    Ok(Json(reactions_for(&app, &p, &c, None, &base(&h)).await?))
}
async fn create_card_reaction(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response> {
    let p = principal(&app, &h, &a).await?;
    let c = card(&app, &p, &n).await?;
    create_reaction(&app, &p, &c, None, &base(&h), body).await
}
async fn delete_card_reaction(
    State(app): Shared,
    Path((a, n, r)): Path<(String, String, String)>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &h, &a).await?;
    let c = card(&app, &p, &n).await?;
    delete_reaction(&app, &p, &c, None, &r).await
}
async fn list_comment_reactions(
    State(app): Shared,
    Path((a, n, comment)): Path<(String, String, String)>,
    h: HeaderMap,
) -> Result<Json<Value>> {
    let p = principal(&app, &h, &a).await?;
    let c = card(&app, &p, &n).await?;
    Ok(Json(
        reactions_for(&app, &p, &c, Some(&comment), &base(&h)).await?,
    ))
}
async fn create_comment_reaction(
    State(app): Shared,
    Path((a, n, comment)): Path<(String, String, String)>,
    h: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response> {
    let p = principal(&app, &h, &a).await?;
    let c = card(&app, &p, &n).await?;
    create_reaction(&app, &p, &c, Some(&comment), &base(&h), body).await
}
async fn delete_comment_reaction(
    State(app): Shared,
    Path((a, n, comment, reaction)): Path<(String, String, String, String)>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &h, &a).await?;
    let c = card(&app, &p, &n).await?;
    delete_reaction(&app, &p, &c, Some(&comment), &reaction).await
}

async fn landing(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let boards: Vec<String> = sqlx::query_scalar(
        "SELECT b.id FROM boards b JOIN accesses x ON x.board_id=b.id WHERE x.user_id=? AND b.account_id=? LIMIT 2",
    )
    .bind(&p.user_id)
    .bind(&p.account_id)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let destination = if boards.len() == 1 {
        format!("/{account}/boards/{}", boards[0])
    } else {
        format!("/{account}/")
    };
    Ok((StatusCode::FOUND, [(header::LOCATION, destination)]).into_response())
}

async fn home() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../static/index.html"))
}
