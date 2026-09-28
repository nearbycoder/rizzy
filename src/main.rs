use axum::{
    Json, Router, ServiceExt,
    body::{Body, Bytes},
    extract::{
        ConnectInfo, Extension, FromRequest, OriginalUri, Path, Query, RawQuery, Request, State,
    },
    http::{HeaderMap, HeaderValue, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Datelike, Local, Timelike, Utc};
use rand::Rng;
use regex::{Regex, RegexBuilder};
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use sqlx::{
    Any as Db, AnyPool as DbPool, Connection, QueryBuilder, Row,
    any::{AnyPoolOptions, AnyRow as DbRow},
    sqlite::{SqliteConnectOptions, SqliteConnection, SqliteJournalMode},
};
use std::{
    collections::{HashMap, HashSet},
    env,
    net::{IpAddr, SocketAddr},
    process::Stdio,
    sync::{Arc, OnceLock, Weak},
    time::{Duration, Instant},
};
use tokio::io::AsyncWriteExt;
use tokio::net::lookup_host;
use tokio::process::Command;
use tower::{
    Layer,
    util::{MapRequestLayer, MapResponseLayer},
};
mod access_tokens_page;
mod account_cancellation;
mod account_sequence;
mod account_settings;
mod account_transfer;
mod action_mailbox;
mod assignment_page;
mod board_page;
mod board_settings;
mod board_views;
mod card_board_page;
mod card_column_page;
mod card_controls;
mod card_live;
mod card_page;
mod cards_page;
mod comment_edit_page;
mod deletion_cleanup;
mod events_page;
mod exports;
mod imports;
mod join_code_page;
mod my_menu;
mod mysql_search;
mod named_column_page;
mod notification_mail;
mod notification_page;
mod passkeys;
mod pins_page;
mod public_pages;
mod push_delivery;
mod push_subscriptions;
mod pwa;
mod reaction_page;
mod recurring_cleanup;
mod rich_text;
mod rich_text_records;
mod search_page;
mod session_transfer;
mod signed_global_id;
mod signup_name;
mod signup_page;
mod signup_seed;
mod storage;
mod tag_cleanup;
mod tagging_page;
mod timezone;
mod unsubscribes;
mod user_page;
mod webhook_cleanup;
mod webhook_page;

struct App {
    db: DbPool,
    mysql: bool,
    qr_secret: String,
    sgid_signer: signed_global_id::Signer,
    write_lock: tokio::sync::Mutex<()>,
    auth_lock: tokio::sync::Mutex<()>,
    notification_html_cache: tokio::sync::Mutex<HashMap<String, NotificationHtmlCacheEntry>>,
    notification_html_build_locks:
        tokio::sync::Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    events_html_cache: tokio::sync::Mutex<HashMap<String, EventsHtmlCacheEntry>>,
    events_html_build_locks: tokio::sync::Mutex<HashMap<String, Weak<tokio::sync::Mutex<()>>>>,
    board_etag_connection: Option<tokio::sync::Mutex<SqliteConnection>>,
    board_etag_cache: tokio::sync::Mutex<HashMap<String, BoardEtagCacheEntry>>,
    export_notify: tokio::sync::Notify,
    import_notify: tokio::sync::Notify,
}

async fn begin_write(app: &App) -> std::result::Result<sqlx::Transaction<'_, Db>, sqlx::Error> {
    if app.mysql {
        app.db.begin().await
    } else {
        app.db.begin_with("BEGIN IMMEDIATE").await
    }
}
async fn lock_board_for_write(
    tx: &mut sqlx::Transaction<'_, Db>,
    board_id: &str,
    mysql: bool,
) -> Result<()> {
    if mysql {
        sqlx::query("SELECT id FROM boards WHERE id=? FOR UPDATE")
            .bind(board_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}
async fn begin_card_board_write<'a>(
    app: &'a App,
    card_id: &str,
    board_id: &str,
) -> Result<(sqlx::Transaction<'a, Db>, String)> {
    let (tx, current_board, _) =
        begin_card_board_write_with_snapshot(app, card_id, board_id, false).await?;
    Ok((tx, current_board))
}
async fn begin_card_board_write_with_snapshot<'a>(
    app: &'a App,
    card_id: &str,
    board_id: &str,
    snapshot: bool,
) -> Result<(sqlx::Transaction<'a, Db>, String, Option<DbRow>)> {
    let mut current_board = board_id.to_owned();
    for _ in 0..16 {
        let mut tx = begin_write(app).await.map_err(db_err)?;
        lock_board_for_write(&mut tx, &current_board, app.mysql).await?;
        let mut current = None;
        if app.mysql {
            // Callers can hold a stale card snapshot after a move. Acquire the
            // board lock first, then lock and recheck the card before writing.
            let actual_board: String = if snapshot {
                let row = sqlx::query("SELECT board_id,title,description,search_text,status,created_at,last_active_at FROM cards WHERE id=? FOR UPDATE")
                    .bind(card_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db_err)?;
                let board = row.get("board_id");
                current = Some(row);
                board
            } else {
                sqlx::query_scalar("SELECT board_id FROM cards WHERE id=? FOR UPDATE")
                    .bind(card_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db_err)?
            };
            if actual_board != current_board {
                tx.rollback().await.map_err(db_err)?;
                current_board = actual_board;
                continue;
            }
        } else if snapshot {
            current = Some(sqlx::query("SELECT board_id,title,description,search_text,status,created_at,last_active_at FROM cards WHERE id=?")
                .bind(card_id)
                .fetch_one(&mut *tx)
                .await
                .map_err(db_err)?);
        }
        return Ok((tx, current_board, current));
    }
    Err(err(StatusCode::CONFLICT, "Card moved during write"))
}
async fn begin_accessible_card_board_write<'a>(
    app: &'a App,
    p: &Principal,
    card_id: &str,
    board_id: &str,
) -> Result<(sqlx::Transaction<'a, Db>, String)> {
    let (mut tx, current_board) = begin_card_board_write(app, card_id, board_id).await?;
    let accessible: Option<String> =
        sqlx::query_scalar("SELECT board_id FROM accesses WHERE board_id=? AND user_id=?")
            .bind(&current_board)
            .bind(&p.user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
    if accessible.is_none() {
        return Err(missing());
    }
    Ok((tx, current_board))
}
async fn begin_accessible_card_board_write_snapshot<'a>(
    app: &'a App,
    p: &Principal,
    card_id: &str,
    board_id: &str,
) -> Result<(sqlx::Transaction<'a, Db>, String, DbRow)> {
    let (mut tx, current_board, current) =
        begin_card_board_write_with_snapshot(app, card_id, board_id, true).await?;
    let accessible: Option<String> =
        sqlx::query_scalar("SELECT board_id FROM accesses WHERE board_id=? AND user_id=?")
            .bind(&current_board)
            .bind(&p.user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
    if accessible.is_none() {
        return Err(missing());
    }
    Ok((tx, current_board, current.expect("snapshot requested")))
}
async fn begin_accessible_board_write<'a>(
    app: &'a App,
    p: &Principal,
    board_id: &str,
) -> Result<sqlx::Transaction<'a, Db>> {
    let mut tx = begin_write(app).await.map_err(db_err)?;
    let board_sql = if app.mysql {
        "SELECT id FROM boards WHERE id=? AND account_id=? FOR UPDATE"
    } else {
        "SELECT id FROM boards WHERE id=? AND account_id=?"
    };
    let board: Option<String> = sqlx::query_scalar(board_sql)
        .bind(board_id)
        .bind(&p.account_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;
    if board.is_none() {
        return Err(missing());
    }
    let accessible: Option<String> =
        sqlx::query_scalar("SELECT board_id FROM accesses WHERE board_id=? AND user_id=?")
            .bind(board_id)
            .bind(&p.user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
    if accessible.is_none() {
        return Err(missing());
    }
    Ok(tx)
}
struct NotificationHtmlCacheEntry {
    version: (i64, Option<String>, i64),
    html: String,
    cached_at: Instant,
}
struct EventsHtmlCacheEntry {
    html: String,
    cached_at: Instant,
}
struct BoardEtagCacheEntry {
    data_version: i64,
    etag: String,
}
type Shared = State<Arc<App>>;
type Result<T> = std::result::Result<T, ApiError>;

#[derive(Debug)]
// The final field allows a handler to retry a transaction after MySQL rolls it back.
struct ApiError(StatusCode, String, bool);
impl ApiError {
    fn is_retryable_deadlock(&self) -> bool {
        self.2
    }
}
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        if self.0 == StatusCode::FOUND {
            return (
                StatusCode::FOUND,
                [
                    (header::LOCATION, self.1),
                    (header::CONTENT_TYPE, "text/html; charset=utf-8".into()),
                ],
            )
                .into_response();
        }
        if self.0 == StatusCode::UNAUTHORIZED && self.1 == "HTTP Token: Access denied." {
            return (
                StatusCode::UNAUTHORIZED,
                [(header::WWW_AUTHENTICATE, "Token realm=\"Application\"")],
                "HTTP Token: Access denied.\n",
            )
                .into_response();
        }
        if self.0 == StatusCode::NOT_FOUND && self.1 == "Not found" {
            return (self.0, Json(json!({"status":404,"error":"Not Found"}))).into_response();
        }
        if self.0 == StatusCode::NOT_ACCEPTABLE && self.1 == "Not Acceptable" {
            return (self.0, Json(json!({"status":406,"error":"Not Acceptable"}))).into_response();
        }
        if self.0 == StatusCode::BAD_REQUEST && self.1 == "Bad Request" {
            return (self.0, Json(json!({"status":400,"error":"Bad Request"}))).into_response();
        }
        if self.0 == StatusCode::INTERNAL_SERVER_ERROR && self.1 == "Internal Server Error" {
            return source_json_internal_error();
        }
        (self.0, Json(json!({"message": self.1}))).into_response()
    }
}
fn err(status: StatusCode, message: &str) -> ApiError {
    ApiError(status, message.into(), false)
}
fn source_json_internal_error() -> Response {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        [(header::CONTENT_TYPE, "application/json; charset=utf-8")],
        r#"{"status":500,"error":"Internal Server Error"}"#,
    )
        .into_response()
}
fn token_denied() -> ApiError {
    err(StatusCode::UNAUTHORIZED, "HTTP Token: Access denied.")
}
#[derive(Clone)]
struct OriginalAccept(Option<HeaderValue>);
#[derive(Clone, Copy)]
struct ShadowedMethodResponse;
fn rewrite_json_uri<B>(mut request: axum::http::Request<B>) -> axum::http::Request<B> {
    // This runs outside the router so collection routes such as /boards.json
    // can match /boards. Explicit client configuration paths have their own routes.
    request.headers_mut().remove("x-rizzy-original-path");
    let original = request.uri().clone();
    let path = original.path();
    if !path.ends_with(".json")
        || path.contains("/client_configurations/")
        || path == "/manifest.json"
        || path == "/error.css"
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
    let original_accept = request.headers().get(header::ACCEPT).cloned();
    request
        .extensions_mut()
        .insert(OriginalAccept(original_accept));
    // OPTIONS has no matching controller route, so its error format follows
    // Accept even when the requested path ends in .json.
    if request.method() != Method::OPTIONS {
        request
            .headers_mut()
            .insert(header::ACCEPT, HeaderValue::from_static("application/json"));
    }
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

async fn buffered_response_etag(request: Request, next: Next) -> Response {
    let conditional = request.headers().get(header::IF_NONE_MATCH).cloned();
    let conditional_method = matches!(request.method(), &Method::GET | &Method::HEAD);
    let response = next.run(request).await;
    if !matches!(response.status(), StatusCode::OK | StatusCode::CREATED)
        || response.headers().contains_key(header::ETAG)
        || response.headers().contains_key(header::LAST_MODIFIED)
        || !response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                value.starts_with("application/json")
                    || value.starts_with("text/html")
                    || value.starts_with("text/vnd.turbo-stream.html")
            })
    {
        return response;
    }
    let (mut parts, body) = response.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, usize::MAX).await else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Could not render response",
        )
            .into_response();
    };
    if bytes.is_empty() {
        return Response::from_parts(parts, Body::from(bytes));
    }
    // Rack::ETag hashes the emitted body and uses the first 32 hex digits.
    let etag = format!("W/\"{}\"", hex::encode(&Sha256::digest(&bytes)[..16]));
    let etag = HeaderValue::from_str(&etag).expect("hex ETag");
    parts.headers.insert(header::ETAG, etag.clone());
    if !parts.headers.contains_key(header::CACHE_CONTROL) {
        parts.headers.insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=0, private, must-revalidate"),
        );
    }
    if conditional_method && parts.status == StatusCode::OK && conditional.as_ref() == Some(&etag) {
        parts.status = StatusCode::NOT_MODIFIED;
        parts.headers.remove(header::CONTENT_TYPE);
        parts.headers.remove(header::CONTENT_LENGTH);
        return Response::from_parts(parts, Body::empty());
    }
    Response::from_parts(parts, Body::from(bytes))
}

async fn redirect_tenanted_public_route(
    State(app): Shared,
    request: Request,
    next: Next,
) -> Response {
    if request.method() == Method::OPTIONS {
        return next.run(request).await;
    }
    let path = request.uri().path();
    let Some((account, rest)) = path.trim_start_matches('/').split_once('/') else {
        return next.run(request).await;
    };
    let Ok(slug) = account.parse::<i64>() else {
        return next.run(request).await;
    };
    let rest = format!("/{rest}");
    let disallowed = rest == "/session"
        || rest.starts_with("/session/")
        || rest == "/signup"
        || rest.starts_with("/signup/")
        || (rest == "/my/identity" && !request.headers().contains_key("x-rizzy-original-path"))
        || (rest == "/my/passkey_challenge" && request.method() == Method::POST)
        || rest == "/account/imports/new"
        || (rest == "/account/imports" && request.method() == Method::POST)
        || rest == "/manifest"
        || rest == "/service-worker";
    if !disallowed {
        return next.run(request).await;
    }
    // These controllers authenticate before their account-scope callback.
    // A signed-out request therefore goes to sign-in even when it carries an
    // account prefix that an authenticated request must not use.
    let requires_auth = rest == "/session/menu"
        || rest == "/signup/completion/new"
        || (rest == "/signup/completion" && request.method() == Method::POST)
        || rest == "/my/identity"
        || rest == "/account/imports/new"
        || (rest == "/account/imports" && request.method() == Method::POST);
    if requires_auth {
        if let Err(error) = identity_auth(&app, request.headers()).await {
            return error.into_response();
        }
    }
    let exists =
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM accounts WHERE external_account_id=?")
            .bind(slug)
            .fetch_one(&app.db)
            .await;
    match exists {
        Ok(0) => next.run(request).await,
        Ok(_) => (
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("{}/{slug}/", base(request.headers())),
            )],
            Html(String::new()),
        )
            .into_response(),
        Err(error) => db_err(error).into_response(),
    }
}

async fn assign_request_id(mut request: Request, next: Next) -> Response {
    let supplied = request
        .headers()
        .get("x-request-id")
        .map(HeaderValue::as_bytes);
    let request_id = match supplied {
        Some(value) if value.iter().any(|byte| !byte.is_ascii_whitespace()) => value
            .iter()
            .copied()
            .filter(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'@'))
            .take(255)
            .map(char::from)
            .collect::<String>(),
        _ => uuid::Uuid::new_v4().to_string(),
    };
    let header_value = HeaderValue::from_str(&request_id).expect("sanitized request id");
    request
        .headers_mut()
        .insert("x-request-id", header_value.clone());
    let mut response = next.run(request).await;
    response.headers_mut().insert("x-request-id", header_value);
    response
}
fn rejected_forgery(headers: &HeaderMap) -> Response {
    if headers.contains_key("x-rizzy-json-request") {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            [(header::CONTENT_TYPE, "application/json; charset=UTF-8")],
            r#"{"status":422,"error":"Unprocessable Content"}"#,
        )
            .into_response()
    } else {
        static_error_page(
            include_str!("../static/422.html"),
            StatusCode::UNPROCESSABLE_ENTITY,
        )
    }
}
async fn prevent_request_forgery(State(app): Shared, mut request: Request, next: Next) -> Response {
    let path = request.uri().path().to_owned();
    let exempt = path.ends_with("/notifications/unsubscribe")
        || path.contains("/collections/")
        || (path == "/signup" && request.method() == Method::GET)
        || path == "/error.css"
        || path == "/manifest.json"
        || path == "/service-worker"
        || path == "/service-worker.js"
        || path.starts_with("/assets/")
        || path.starts_with("/app-icon");
    if exempt {
        return next.run(request).await;
    }
    if request.method() == Method::POST && path.ends_with("/my/passkey_challenge") {
        let valid_header = valid_passkey_csrf(&app, request.headers());
        if !valid_header {
            let form = request
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"));
            if !form {
                return rejected_forgery(request.headers());
            }
            let (parts, body) = request.into_parts();
            let Ok(bytes) = axum::body::to_bytes(body, 1024 * 1024).await else {
                return rejected_forgery(&parts.headers);
            };
            let valid_form = url::form_urlencoded::parse(&bytes)
                .filter(|(key, _)| key == "authenticity_token")
                .any(|(_, value)| valid_passkey_csrf_value(&app, &parts.headers, &value));
            request = Request::from_parts(parts, Body::from(bytes));
            if !valid_form {
                return rejected_forgery(request.headers());
            }
        }
    }
    let headers = request.headers();
    let safe = matches!(request.method().as_str(), "GET" | "HEAD" | "QUERY");
    if !safe {
        let origin_matches = match headers.get(header::ORIGIN) {
            None => true,
            Some(origin) => origin.to_str().is_ok_and(|origin| origin == base(headers)),
        };
        let site = headers
            .get("sec-fetch-site")
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .map(str::to_ascii_lowercase);
        let site_allowed = match site.as_deref() {
            Some("same-origin" | "same-site") => true,
            None => {
                headers.contains_key("x-rizzy-json-request")
                    || (!secure_request(headers)
                        && env::var("RIZZY_FORCE_SSL").as_deref() != Ok("1")
                        && !env::var("RIZZY_PUBLIC_URL")
                            .is_ok_and(|url| url.starts_with("https://")))
            }
            _ => false,
        };
        if !origin_matches || !site_allowed {
            return rejected_forgery(headers);
        }
    }
    let mut response = next.run(request).await;
    if matches!(
        response.status(),
        StatusCode::NOT_FOUND | StatusCode::NOT_ACCEPTABLE
    ) || authentication_redirect(&response)
    {
        return response;
    }
    let vary = response
        .headers()
        .get(header::VARY)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    if !vary
        .split(',')
        .any(|value| value.trim().eq_ignore_ascii_case("Sec-Fetch-Site"))
    {
        let new_vary = if vary.is_empty() {
            "Sec-Fetch-Site".to_owned()
        } else {
            format!("{vary}, Sec-Fetch-Site")
        };
        if let Ok(value) = HeaderValue::from_str(&new_vary) {
            response.headers_mut().insert(header::VARY, value);
        }
    }
    response
}
async fn block_search_engine_indexing(request: Request, next: Next) -> Response {
    let application_route = application_controller_route(request.uri().path(), request.method());
    let mut response = next.run(request).await;
    if application_route
        && !matches!(
            response.status(),
            StatusCode::NOT_FOUND | StatusCode::NOT_ACCEPTABLE | StatusCode::INTERNAL_SERVER_ERROR
        )
        && !authentication_redirect(&response)
    {
        response
            .headers_mut()
            .insert("x-robots-tag", HeaderValue::from_static("none"));
    }
    response
}

async fn html_error_contract(request: Request, next: Next) -> Response {
    let path = request.uri().path();
    // The source route declares defaults: { format: :json } for webhook deliveries.
    let route_defaults_json = path.contains("/webhooks/") && path.ends_with("/deliveries");
    let wants_html = request
        .headers()
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/html"))
        && !path.ends_with(".json")
        && !route_defaults_json;
    let response = next.run(request).await;
    if !wants_html
        || !response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/json"))
    {
        return response;
    }
    match response.status() {
        StatusCode::NOT_FOUND => {
            static_error_page(include_str!("../static/404.html"), StatusCode::NOT_FOUND)
        }
        StatusCode::NOT_ACCEPTABLE => {
            (StatusCode::NOT_ACCEPTABLE, Html(String::new())).into_response()
        }
        _ => response,
    }
}
fn authentication_redirect(response: &Response) -> bool {
    response.status() == StatusCode::FOUND
        && response
            .headers()
            .get(header::LOCATION)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|location| {
                location.ends_with("/session/new") || location.ends_with("/session/menu")
            })
}
async fn signup_redirect(headers: HeaderMap) -> Response {
    (
        StatusCode::MOVED_PERMANENTLY,
        [(header::LOCATION, format!("{}/signup/new", base(&headers)))],
    )
        .into_response()
}
fn application_controller_route(path: &str, method: &Method) -> bool {
    path != "/up"
        && path != "/error.css"
        && path != "/manifest.json"
        && !path.starts_with("/assets/")
        && !path.starts_with("/app-icon")
        && !path.contains("/rails/active_storage/")
        && !path.contains("/collections/")
        && !(path == "/signup" && method == Method::GET)
}
fn user_agent_version(agent: &str, product: &str) -> Option<(u32, u32)> {
    let version: String = agent
        .split_once(product)?
        .1
        .chars()
        .take_while(|character| character.is_ascii_digit() || *character == '.')
        .collect();
    let mut numbers = version.split('.');
    let major = numbers.next()?.parse().ok()?;
    let minor = numbers
        .next()
        .and_then(|part| part.parse().ok())
        .unwrap_or(0);
    Some((major, minor))
}
fn unsupported_browser(agent: &str) -> bool {
    let lower = agent.to_ascii_lowercase();
    if lower.contains("bot")
        || lower.contains("googleimageproxy")
        || lower.contains("chrome-lighthouse")
        || [
            "Edg/",
            "EdgA/",
            "EdgiOS/",
            "Edge/",
            "Vivaldi/",
            "baidubrowser/",
            "MicroMessenger/",
        ]
        .iter()
        .any(|product| agent.contains(product))
    {
        return false;
    }
    if let Some(version) =
        user_agent_version(agent, "MSIE ").or_else(|| user_agent_version(agent, "rv:"))
    {
        if agent.contains("MSIE ") || agent.contains("Trident/") {
            return version > (0, 0);
        }
    }
    if let Some(version) = user_agent_version(agent, "OPR/").or_else(|| {
        agent.contains("Opera/").then(|| {
            user_agent_version(agent, "Version/").or_else(|| user_agent_version(agent, "Opera/"))
        })?
    }) {
        return version < (106, 0);
    }
    if let Some(version) = user_agent_version(agent, "Firefox/") {
        return version < (121, 0);
    }
    if let Some(version) = user_agent_version(agent, "HeadlessChrome/")
        .or_else(|| user_agent_version(agent, "Chrome/"))
        .or_else(|| user_agent_version(agent, "CriOS/"))
    {
        return version < (120, 0);
    }
    if agent.contains("Safari/") {
        if let Some(version) = user_agent_version(agent, "Version/") {
            return version < (17, 2);
        }
        if agent.contains("iPhone") || agent.contains("iPad") {
            if let Some(version) = agent.split(" OS ").nth(1) {
                let normalized: String = version
                    .chars()
                    .take_while(|character| {
                        character.is_ascii_digit() || matches!(character, '_' | '.')
                    })
                    .map(|character| if character == '_' { '.' } else { character })
                    .collect();
                let mut parts = normalized.split('.');
                if let Some(major) = parts.next().and_then(|part| part.parse::<u32>().ok()) {
                    let minor = parts
                        .next()
                        .and_then(|part| part.parse::<u32>().ok())
                        .unwrap_or(0);
                    return (major, minor) < (17, 2);
                }
            }
        }
    }
    false
}
fn browser_access_is_public(path: &str, method: &Method) -> bool {
    (path == "/session/new" && method == Method::GET)
        || (path == "/session" && method == Method::POST)
        || (path.starts_with("/session/magic_link")
            && matches!(*method, Method::GET | Method::POST))
        || (path == "/session/passkey" && method == Method::POST)
        || (path.starts_with("/session/transfers/")
            && matches!(*method, Method::GET | Method::PATCH | Method::PUT))
        || (path == "/signup/new" && method == Method::GET)
        || (path == "/signup" && method == Method::POST)
        || (path.starts_with("/client_configurations/") && method == Method::GET)
        || (path.starts_with("/join_codes/") && method == Method::GET)
        || (path.starts_with("/qr_codes/") && method == Method::GET)
        || (path.contains("/public/") && method == Method::GET)
        || path.ends_with("/notifications/unsubscribe")
        || (path.ends_with("/my/passkey_challenge") && method == Method::POST)
}
async fn allow_modern_browser(State(app): Shared, request: Request, next: Next) -> Response {
    let path = request.uri().path();
    let blocked = application_controller_route(path, request.method())
        && request
            .headers()
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(unsupported_browser);
    if blocked
        && (browser_access_is_public(path, request.method())
            || identity_auth(&app, request.headers()).await.is_ok())
    {
        let mut response = (
            StatusCode::NOT_ACCEPTABLE,
            [
                (header::CACHE_CONTROL, "no-cache"),
                (header::CONTENT_SECURITY_POLICY, STATIC_CSP),
            ],
            Html(include_str!("../static/406-unsupported-browser.html")),
        )
            .into_response();
        if request.headers().contains_key(header::ACCEPT) {
            response
                .headers_mut()
                .insert(header::VARY, HeaderValue::from_static("Accept"));
        }
        return response;
    }
    next.run(request).await
}
fn platform_user_agent(headers: &HeaderMap) -> String {
    if let Some(cookie) = cookie_value(headers, "x_user_agent") {
        let decoded =
            url::form_urlencoded::parse(format!("agent={}", cookie.trim_matches('"')).as_bytes())
                .next()
                .map(|(_, value)| value.into_owned())
                .unwrap_or_default();
        if !decoded.trim().is_empty() {
            return decoded;
        }
    }
    headers
        .get(header::USER_AGENT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}
fn public_layout_path(path: &str) -> bool {
    path == "/session/new"
        || path == "/session/menu"
        || path.starts_with("/session/magic_link")
        || path.starts_with("/signup")
        || path.starts_with("/join_codes/")
        || path.contains("/public/")
        || path.contains("/users/joins")
        || path.contains("/users/verifications")
        || path.starts_with("/account/imports")
}
fn platform_attributes(agent: &str, public_layout: bool) -> String {
    let ios = agent.contains("iPhone") || agent.contains("iPad");
    let android = agent.contains("Android");
    let native = agent.contains("Hotwire Native");
    let platform = if native && android {
        "native android"
    } else if native && ios {
        "native ios"
    } else if ios || android {
        "mobile web"
    } else {
        "desktop web"
    };
    if public_layout {
        return format!(" data-platform=\"{platform}\"");
    }
    let bridge = if native && android {
        "android"
    } else if native && ios {
        "ios"
    } else {
        ""
    };
    let components = if native {
        agent
            .split_once("bridge-components: [")
            .and_then(|(_, rest)| rest.split_once(']'))
            .map(|(components, _)| components)
            .unwrap_or_default()
    } else {
        ""
    };
    format!(
        " data-platform=\"{platform}\" data-bridge-platform=\"{bridge}\" data-bridge-components=\"{}\"",
        escape(components)
    )
}
async fn register_pwa_on_pages(State(app): Shared, request: Request, next: Next) -> Response {
    let static_error_page = matches!(
        request.uri().path(),
        "/400.html" | "/404.html" | "/406-unsupported-browser.html" | "/422.html" | "/500.html"
    );
    let framework_error_page = request
        .uri()
        .path()
        .starts_with("/rails/conductor/action_mailbox/");
    let session_cookie = cookie_value(request.headers(), "session_token").is_some();
    let agent = platform_user_agent(request.headers());
    let public_layout = public_layout_path(request.uri().path());
    let session_headers = if session_cookie {
        Some(request.headers().clone())
    } else {
        None
    };
    let response = next.run(request).await;
    if static_error_page || framework_error_page {
        return response;
    }
    if matches!(
        response.status(),
        StatusCode::NOT_FOUND | StatusCode::INTERNAL_SERVER_ERROR
    ) {
        return response;
    }
    if !response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("text/html"))
    {
        return response;
    }
    let successful = response.status().is_success();
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
    let Ok(mut html) = String::from_utf8(bytes.to_vec()) else {
        return Response::from_parts(parts, Body::from(bytes));
    };
    if let Some(body) = html
        .find("<body")
        .and_then(|start| html[start..].find('>').map(|end| start + end))
    {
        html.insert_str(body, &platform_attributes(&agent, public_layout));
    }
    if let Some(end) = html.find("</body>") {
        html.insert_str(end, include_str!("../assets/timezone_cookie.html"));
    }
    let register_pwa = if successful {
        if let Some(headers) = &session_headers {
            identity_auth(&app, headers).await.is_ok()
        } else {
            false
        }
    } else {
        false
    };
    if register_pwa {
        if let Some(head) = html.find("</head>") {
            html.insert_str(head, "<link rel=\"manifest\" href=\"/manifest.json\">");
        }
        let script_position = html
            .find("</body>")
            .or_else(|| html.find("</html>"))
            .unwrap_or(html.len());
        html.insert_str(script_position, include_str!("../assets/pwa_register.html"));
    }
    parts.headers.remove(header::CONTENT_LENGTH);
    Response::from_parts(parts, Body::from(html))
}
#[track_caller]
fn db_err(e: sqlx::Error) -> ApiError {
    // SQLx exposes MySQL's SQLSTATE, not its numeric 1213 error number.
    let deadlock =
        matches!(&e, sqlx::Error::Database(error) if error.code().as_deref() == Some("40001"));
    eprintln!("database error at {}: {e}", std::panic::Location::caller());
    ApiError(
        StatusCode::INTERNAL_SERVER_ERROR,
        "Database error".into(),
        deadlock,
    )
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
fn json_millisecond_time(mut raw: String) -> String {
    if raw.len() == 27
        && raw.is_ascii()
        && raw.as_bytes()[19] == b'.'
        && raw.as_bytes()[20..26].iter().all(u8::is_ascii_digit)
        && raw.ends_with('Z')
    {
        raw.truncate(23);
        raw.push('Z');
    }
    raw
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
    tx: &mut sqlx::Transaction<'_, Db>,
    account_id: &str,
    mysql: bool,
) -> std::result::Result<(), sqlx::Error> {
    loop {
        let timestamp = now();
        let statement = if mysql {
            "INSERT IGNORE INTO account_join_codes(id,account_id,code,created_at,updated_at) VALUES(?,?,?,?,?)"
        } else {
            "INSERT OR IGNORE INTO account_join_codes(id,account_id,code,created_at,updated_at) VALUES(?,?,?,?,?)"
        };
        let inserted = sqlx::query(statement)
            .bind(id())
            .bind(account_id)
            .bind(join_code())
            .bind(&timestamp)
            .bind(&timestamp)
            .execute(&mut **tx)
            .await?;
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

fn database_pool_size() -> u32 {
    env::var("RIZZY_DB_POOL_SIZE")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(12)
        .clamp(1, 64)
}

async fn resume_mysql_schema(
    db: &DbPool,
    schema: &str,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let mut tables: HashSet<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables WHERE table_schema=DATABASE()",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .collect();
    let mut indexes: HashSet<(String, String)> = sqlx::query(
        "SELECT DISTINCT table_name,index_name FROM information_schema.statistics WHERE table_schema=DATABASE()",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|row| (row.get(0), row.get(1)))
    .collect();
    let mut foreign_keys: HashSet<(String, String)> = sqlx::query(
        "SELECT table_name,constraint_name FROM information_schema.referential_constraints WHERE constraint_schema=DATABASE()",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|row| (row.get(0), row.get(1)))
    .collect();
    let mut generated: HashSet<(String, String)> = sqlx::query(
        "SELECT table_name,column_name FROM information_schema.columns WHERE table_schema=DATABASE() AND generation_expression<>''",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .map(|row| (row.get(0), row.get(1)))
    .collect();
    let statements = schema
        .lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join("\n");
    for statement in statements
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let quoted = |index: usize| -> std::result::Result<&str, Box<dyn std::error::Error>> {
            statement
                .split('`')
                .nth(index)
                .ok_or_else(|| format!("malformed MySQL schema statement: {statement}").into())
        };
        if statement.starts_with("SET NAMES ") {
            sqlx::query(statement).execute(db).await?;
        } else if statement.starts_with("CREATE TABLE ") {
            let table = quoted(1)?;
            if tables.insert(table.to_owned()) {
                sqlx::raw_sql(statement).execute(db).await?;
            }
        } else if statement.starts_with("CREATE INDEX ")
            || statement.starts_with("CREATE UNIQUE INDEX ")
        {
            let index = quoted(1)?;
            let table = quoted(3)?;
            if indexes.insert((table.to_owned(), index.to_owned())) {
                sqlx::query(statement).execute(db).await?;
            }
        } else if let Some((_, definitions)) = statement.split_once(" ADD COLUMN ") {
            let table = quoted(1)?;
            for definition in definitions.split(", ADD COLUMN ") {
                let column = definition
                    .split('`')
                    .nth(1)
                    .ok_or("malformed MySQL generated column")?;
                if generated.insert((table.to_owned(), column.to_owned())) {
                    let ddl = format!("ALTER TABLE `{table}` ADD COLUMN {definition}");
                    sqlx::query(&ddl).execute(db).await?;
                }
            }
        } else if statement.starts_with("ALTER TABLE ") && statement.contains(" ADD CONSTRAINT ") {
            let table = quoted(1)?;
            let constraint = quoted(3)?;
            if foreign_keys.insert((table.to_owned(), constraint.to_owned())) {
                sqlx::query(statement).execute(db).await?;
            }
        } else {
            return Err(format!("unsupported MySQL schema statement: {statement}").into());
        }
    }
    Ok(())
}

async fn validate_mysql_schema(
    db: &DbPool,
    schema: &str,
) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let expected_tables: HashSet<String> = schema
        .lines()
        .filter(|line| line.starts_with("CREATE TABLE "))
        .filter_map(|line| line.split('`').nth(1).map(str::to_owned))
        .collect();
    let expected_indexes: HashSet<String> = schema
        .lines()
        .filter(|line| {
            line.starts_with("CREATE INDEX ") || line.starts_with("CREATE UNIQUE INDEX ")
        })
        .filter_map(|line| line.split('`').nth(1).map(str::to_owned))
        .collect();
    let expected_foreign_keys: HashSet<String> = schema
        .lines()
        .filter(|line| line.starts_with("ALTER TABLE ") && line.contains(" ADD CONSTRAINT "))
        .filter_map(|line| line.split('`').nth(3).map(str::to_owned))
        .collect();
    let expected_generated: HashSet<String> = schema
        .lines()
        .filter(|line| line.starts_with("ALTER TABLE ") && line.contains(" ADD COLUMN "))
        .flat_map(|line| line.split("ADD COLUMN `").skip(1))
        .filter_map(|part| part.split('`').next().map(str::to_owned))
        .collect();
    let actual_tables: HashSet<String> = sqlx::query_scalar(
        "SELECT table_name FROM information_schema.tables WHERE table_schema=DATABASE()",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .collect();
    let actual_indexes: HashSet<String> = sqlx::query_scalar(
        "SELECT DISTINCT index_name FROM information_schema.statistics WHERE table_schema=DATABASE()",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .collect();
    let actual_foreign_keys: HashSet<String> = sqlx::query_scalar(
        "SELECT constraint_name FROM information_schema.referential_constraints WHERE constraint_schema=DATABASE()",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .collect();
    let actual_generated: HashSet<String> = sqlx::query_scalar(
        "SELECT column_name FROM information_schema.columns WHERE table_schema=DATABASE() AND generation_expression<>''",
    )
    .fetch_all(db)
    .await?
    .into_iter()
    .collect();
    for (kind, expected, actual) in [
        ("tables", expected_tables, actual_tables),
        ("indexes", expected_indexes, actual_indexes),
        ("foreign keys", expected_foreign_keys, actual_foreign_keys),
        ("generated columns", expected_generated, actual_generated),
    ] {
        let mut missing: Vec<String> = expected.difference(&actual).cloned().collect();
        missing.sort();
        if !missing.is_empty() {
            let sample = missing
                .iter()
                .take(8)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "incomplete MySQL schema: missing {} {kind} (first: {sample})",
                missing.len()
            )
            .into());
        }
    }
    let expected_checks = schema.matches(" CHECK(").count() as i64;
    let actual_checks: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.table_constraints WHERE table_schema=DATABASE() AND constraint_type='CHECK'",
    )
    .fetch_one(db)
    .await?;
    if actual_checks < expected_checks {
        return Err(format!(
            "incomplete MySQL schema: expected {expected_checks} checks, found {actual_checks}"
        )
        .into());
    }
    Ok(())
}

async fn backfill_webhook_trackers(db: &DbPool, mysql: bool) -> sqlx::Result<()> {
    let insert = if mysql {
        "INSERT IGNORE"
    } else {
        "INSERT OR IGNORE"
    };
    let statement = format!(
        "{insert} INTO webhook_delinquency_trackers(id,account_id,webhook_id,consecutive_failures_count,first_failure_at,created_at,updated_at) SELECT w.id,w.account_id,w.id,w.consecutive_failures_count,w.first_failure_at,w.created_at,w.updated_at FROM webhooks w LEFT JOIN webhook_delinquency_trackers t ON t.webhook_id=w.id WHERE t.id IS NULL"
    );
    sqlx::query(&statement).execute(db).await?;
    Ok(())
}

async fn mysql_setup(url: &str) -> std::result::Result<DbPool, Box<dyn std::error::Error>> {
    let db = AnyPoolOptions::new()
        .max_connections(database_pool_size())
        .connect(url)
        .await?;
    // A dedicated connection keeps the server-wide lock while the pool runs DDL.
    // Concurrent processes must not both decide that an empty schema needs setup.
    let mut setup_lock = sqlx::AnyConnection::connect(url).await?;
    let locked: Option<i64> = sqlx::query_scalar("SELECT GET_LOCK('rizzy_schema_init',30)")
        .fetch_one(&mut setup_lock)
        .await?;
    if locked != Some(1) {
        return Err("Timed out waiting for MySQL schema initialization lock".into());
    }
    let schema = include_str!("../schema.mysql.sql");
    resume_mysql_schema(&db, schema).await?;
    account_sequence::initialize(&db, true).await?;
    backfill_webhook_trackers(&db, true).await?;
    let request_id_capacity: Option<i64> = sqlx::query_scalar(
        "SELECT CAST(character_maximum_length AS SIGNED) FROM information_schema.columns WHERE table_schema=DATABASE() AND table_name='storage_entries' AND column_name='request_id'",
    )
    .fetch_optional(&db)
    .await?;
    if request_id_capacity.is_some_and(|length| length < 255) {
        sqlx::query("ALTER TABLE storage_entries MODIFY COLUMN request_id VARCHAR(255) NULL")
            .execute(&db)
            .await?;
    }
    let signed_id_capacity: Option<i64> = sqlx::query_scalar(
        "SELECT CAST(character_maximum_length AS SIGNED) FROM information_schema.columns WHERE table_schema=DATABASE() AND table_name='storage_blobs' AND column_name='signed_id'",
    )
    .fetch_optional(&db)
    .await?;
    if signed_id_capacity.is_some_and(|length| length < 255) {
        sqlx::query("ALTER TABLE storage_blobs MODIFY COLUMN signed_id VARCHAR(255) NOT NULL")
            .execute(&db)
            .await?;
    }
    let bulk_touch_epoch_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema=DATABASE() AND table_name='cards' AND column_name='bulk_touch_epoch'",
    )
    .fetch_one(&db)
    .await?;
    if bulk_touch_epoch_exists == 0 {
        sqlx::query("ALTER TABLE cards ADD COLUMN bulk_touch_epoch BIGINT NOT NULL DEFAULT 0")
            .execute(&db)
            .await?;
    }
    let assignment_updated_at_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema=DATABASE() AND table_name='assignments' AND column_name='updated_at'",
    )
    .fetch_one(&db)
    .await?;
    if assignment_updated_at_exists == 0 {
        sqlx::query(
            "ALTER TABLE assignments ADD COLUMN updated_at VARCHAR(40) NOT NULL DEFAULT ''",
        )
        .execute(&db)
        .await?;
    }
    let assignment_update_backfilled: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM app_secrets WHERE name='assignments_updated_at_v1')",
    )
    .fetch_one(&db)
    .await?;
    if assignment_updated_at_exists == 0 || assignment_update_backfilled == 0 {
        sqlx::query("UPDATE assignments SET updated_at=created_at WHERE updated_at=''")
            .execute(&db)
            .await?;
        sqlx::query("INSERT INTO app_secrets(name,value) VALUES('assignments_updated_at_v1','done') ON DUPLICATE KEY UPDATE value='done'")
            .execute(&db)
            .await?;
    }
    let tag_updated_at_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema=DATABASE() AND table_name='tags' AND column_name='updated_at'",
    )
    .fetch_one(&db)
    .await?;
    if tag_updated_at_exists == 0 {
        sqlx::query("ALTER TABLE tags ADD COLUMN updated_at VARCHAR(40) NOT NULL DEFAULT ''")
            .execute(&db)
            .await?;
    }
    let tag_records_backfilled: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM app_secrets WHERE name='tag_records_v1')")
            .fetch_one(&db)
            .await?;
    for (name, definition) in [
        ("id", "VARCHAR(64) NOT NULL DEFAULT ''"),
        ("created_at", "VARCHAR(40) NOT NULL DEFAULT ''"),
        ("updated_at", "VARCHAR(40) NOT NULL DEFAULT ''"),
    ] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema=DATABASE() AND table_name='taggings' AND column_name=?",
        )
        .bind(name)
        .fetch_one(&db)
        .await?;
        if exists == 0 {
            sqlx::query(&format!(
                "ALTER TABLE taggings ADD COLUMN `{name}` {definition}"
            ))
            .execute(&db)
            .await?;
        }
    }
    if tag_records_backfilled == 0 {
        sqlx::query("UPDATE tags SET updated_at=created_at WHERE updated_at=''")
            .execute(&db)
            .await?;
        sqlx::query(
            "UPDATE taggings SET id=CONCAT('0',SUBSTRING(REPLACE(UUID(),'-',''),1,24)) WHERE id=''",
        )
        .execute(&db)
        .await?;
        sqlx::query("UPDATE taggings t LEFT JOIN cards c ON c.id=t.card_id SET t.created_at=COALESCE(c.created_at,?) WHERE t.created_at=''")
            .bind(now()).execute(&db).await?;
        sqlx::query("UPDATE taggings SET updated_at=created_at WHERE updated_at=''")
            .execute(&db)
            .await?;
    }
    let tagging_id_index_exists: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM information_schema.statistics WHERE table_schema=DATABASE() AND table_name='taggings' AND index_name='taggings_id'",
    )
    .fetch_one(&db)
    .await?;
    if tagging_id_index_exists == 0 {
        sqlx::query("CREATE UNIQUE INDEX taggings_id ON taggings(id)")
            .execute(&db)
            .await?;
    }
    if tag_records_backfilled == 0 {
        sqlx::query("INSERT INTO app_secrets(name,value) VALUES('tag_records_v1','done') ON DUPLICATE KEY UPDATE value='done'")
            .execute(&db).await?;
    }
    let interaction_records_backfilled: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM app_secrets WHERE name='pin_reaction_update_v1')",
    )
    .fetch_one(&db)
    .await?;
    for table in ["pins", "reactions"] {
        let exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM information_schema.columns WHERE table_schema=DATABASE() AND table_name=? AND column_name='updated_at'",
        )
        .bind(table)
        .fetch_one(&db)
        .await?;
        if exists == 0 {
            sqlx::query(&format!(
                "ALTER TABLE `{table}` ADD COLUMN updated_at VARCHAR(40) NOT NULL DEFAULT ''"
            ))
            .execute(&db)
            .await?;
        }
        if interaction_records_backfilled == 0 {
            sqlx::query(&format!(
                "UPDATE `{table}` SET updated_at=created_at WHERE updated_at=''"
            ))
            .execute(&db)
            .await?;
        }
    }
    if interaction_records_backfilled == 0 {
        sqlx::query("INSERT INTO app_secrets(name,value) VALUES('pin_reaction_update_v1','done') ON DUPLICATE KEY UPDATE value='done'")
            .execute(&db).await?;
    }
    validate_mysql_schema(&db, schema).await?;
    mysql_search::setup(&db).await?;
    let bulk_touch_triggers_installed: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM app_secrets WHERE name='cards_bulk_touch_triggers_v3')",
    )
    .fetch_one(&db)
    .await?;
    if bulk_touch_triggers_installed == 0 {
        for name in ["cards_touch_board_update", "cards_touch_column_update"] {
            sqlx::raw_sql(&format!("DROP TRIGGER IF EXISTS {name}"))
                .execute(&db)
                .await?;
        }
    }
    let account_entropy_trigger_installed: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM app_secrets WHERE name='accounts_entropy_bulk_touch_v2')",
    )
    .fetch_one(&db)
    .await?;
    if account_entropy_trigger_installed == 0 {
        sqlx::raw_sql("DROP TRIGGER IF EXISTS accounts_after_update")
            .execute(&db)
            .await?;
    }
    let installed: HashSet<String> = sqlx::query_scalar(
        "SELECT trigger_name FROM information_schema.triggers WHERE trigger_schema=DATABASE()",
    )
    .fetch_all(&db)
    .await?
    .into_iter()
    .collect();
    for section in include_str!("../schema.mysql.triggers.sql")
        .split("-- trigger: ")
        .skip(1)
    {
        let (name, ddl) = section
            .split_once('\n')
            .ok_or("Malformed MySQL trigger section")?;
        let ddl = ddl.trim();
        if ddl.is_empty() {
            return Err(format!("Empty MySQL trigger section: {}", name.trim()).into());
        }
        if !installed.contains(name.trim()) {
            sqlx::raw_sql(ddl).execute(&db).await?;
        }
    }
    if bulk_touch_triggers_installed == 0 {
        sqlx::query("INSERT INTO app_secrets(name,value) VALUES('cards_bulk_touch_triggers_v3','done') ON DUPLICATE KEY UPDATE value='done'")
            .execute(&db)
            .await?;
    }
    if account_entropy_trigger_installed == 0 {
        sqlx::query("INSERT INTO app_secrets(name,value) VALUES('accounts_entropy_bulk_touch_v2','done') ON DUPLICATE KEY UPDATE value='done'")
            .execute(&db)
            .await?;
    }
    let backfilled: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM app_secrets WHERE name='mysql_backfill_v1')",
    )
    .fetch_one(&db)
    .await?;
    if backfilled == 0 {
        let mut tx = db.begin().await?;
        sqlx::raw_sql(include_str!("../schema.mysql.backfill.sql"))
            .execute(&mut *tx)
            .await?;
        sqlx::query("INSERT INTO app_secrets(name,value) VALUES('mysql_backfill_v1','done')")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
    }
    let rich_text_backfilled: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM app_secrets WHERE name='rich_text_records_backfill_v1')",
    )
    .fetch_one(&db)
    .await?;
    if rich_text_backfilled == 0 {
        rich_text_records::backfill(&db, true).await?;
        sqlx::query(
            "INSERT INTO app_secrets(name,value) VALUES('rich_text_records_backfill_v1','done')",
        )
        .execute(&db)
        .await?;
    }
    let missing_join_codes = sqlx::query("SELECT a.id FROM accounts a WHERE NOT EXISTS(SELECT 1 FROM account_join_codes j WHERE j.account_id=a.id)")
        .fetch_all(&db)
        .await?;
    for account in missing_join_codes {
        let mut tx = db.begin().await?;
        insert_join_code_tx(&mut tx, &account.get::<String, _>("id"), true).await?;
        tx.commit().await?;
    }
    sqlx::query("INSERT IGNORE INTO app_secrets(name,value) VALUES('qr_codes',?)")
        .bind(random_token())
        .execute(&db)
        .await?;
    sqlx::query("SELECT RELEASE_LOCK('rizzy_schema_init')")
        .execute(&mut setup_lock)
        .await?;
    Ok(db)
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
fn passkey_csrf_signature(app: &App, headers: &HeaderMap, token: &str) -> String {
    let session = cookie_value(headers, "session_token").unwrap_or_default();
    hmac_sha256(&app.qr_secret, &format!("passkey-csrf:{session}:{token}"))
}
fn passkey_csrf_from_cookie(app: &App, headers: &HeaderMap) -> Option<String> {
    let cookie = cookie_value(headers, "passkey_csrf")?;
    let (token, signature) = cookie.split_once('.')?;
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let expected = passkey_csrf_signature(app, headers, token);
    if signature.len() != expected.len()
        || signature
            .bytes()
            .zip(expected.bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            != 0
    {
        return None;
    }
    Some(token.to_owned())
}
fn passkey_csrf_token(app: &App, headers: &HeaderMap) -> String {
    passkey_csrf_from_cookie(app, headers).unwrap_or_else(random_token)
}
fn set_passkey_csrf_cookie(
    app: &App,
    request_headers: &HeaderMap,
    response_headers: &mut HeaderMap,
    token: &str,
) {
    let signature = passkey_csrf_signature(app, request_headers, token);
    set_cookie(
        response_headers,
        "passkey_csrf",
        &format!("{token}.{signature}"),
        12 * 3600,
        secure_request(request_headers),
    );
}
fn valid_passkey_csrf(app: &App, headers: &HeaderMap) -> bool {
    headers
        .get("x-csrf-token")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|provided| valid_passkey_csrf_value(app, headers, provided))
}
fn valid_passkey_csrf_value(app: &App, headers: &HeaderMap, provided: &str) -> bool {
    let Some(token) = passkey_csrf_from_cookie(app, headers) else {
        return false;
    };
    provided.len() == token.len()
        && provided
            .bytes()
            .zip(token.bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            == 0
}
fn etag_timezone(headers: &HeaderMap) -> Option<String> {
    let cookie = cookie_value(headers, "timezone")?;
    let value = cookie.trim_matches('"').replace('+', " ");
    let decoded = percent_encoding::percent_decode_str(&value)
        .decode_utf8()
        .ok()?;
    timezone::resolve(&decoded).map(|_| decoded.into_owned())
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
fn card_params(body: &Value) -> Result<&Value> {
    let card = nested(body, "card");
    let fields = card
        .as_object()
        .ok_or_else(|| err(StatusCode::BAD_REQUEST, "Bad Request"))?;
    if !fields.iter().any(|(key, value)| {
        matches!(
            key.as_str(),
            "title" | "description" | "image" | "created_at" | "last_active_at"
        ) && !matches!(value, Value::Array(_) | Value::Object(_))
    }) {
        return Err(err(StatusCode::BAD_REQUEST, "Bad Request"));
    }
    Ok(card)
}
fn card_title_param(card: &Value) -> Option<String> {
    match card.get("title") {
        None => None,
        Some(Value::Null) => Some(String::new()),
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        Some(Value::Bool(value)) => Some(if *value { "t" } else { "f" }.into()),
        Some(_) => None,
    }
}
fn card_description_param(card: &Value) -> Option<String> {
    match card.get("description") {
        None => None,
        Some(Value::Null) => Some(String::new()),
        Some(Value::String(value)) => Some(value.clone()),
        Some(Value::Number(value)) => Some(value.to_string()),
        Some(Value::Bool(value)) => Some(value.to_string()),
        Some(_) => None,
    }
}
fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn required(v: &Value, key: &str) -> Result<String> {
    string(v, key)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| bad(&format!("{key} is required")))
}
fn comment_body_param(v: &Value) -> Result<Option<String>> {
    match v.get("body") {
        Some(Value::String(body)) => Ok(Some(body.clone())),
        Some(Value::Null) => Ok(None),
        _ => Err(err(StatusCode::BAD_REQUEST, "body is required")),
    }
}
fn timestamp_param(v: &Value, key: &str) -> Result<Option<String>> {
    match v.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => chrono::DateTime::parse_from_rfc3339(raw)
            .map(|value| {
                Some(
                    value
                        .with_timezone(&Utc)
                        .to_rfc3339_opts(chrono::SecondsFormat::Micros, true),
                )
            })
            .map_err(|_| bad(&format!("Invalid {key}"))),
        Some(_) => Err(bad(&format!("Invalid {key}"))),
    }
}
fn card_timestamp_param(
    v: &Value,
    key: &str,
    creating: bool,
    mysql: bool,
) -> Result<Option<String>> {
    match v.get(key) {
        None => Ok(None),
        Some(Value::Null) if creating => Ok(None),
        Some(Value::Null) => Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
        )),
        Some(Value::String(raw)) => match parse_card_timestamp(raw) {
            Some(value) => Ok(Some(value)),
            None if creating => Ok(None),
            None => Err(err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Internal Server Error",
            )),
        },
        Some(Value::Array(_) | Value::Object(_)) => Ok(None),
        Some(Value::Bool(false)) if creating => Ok(None),
        Some(Value::Bool(_) | Value::Number(_)) if mysql => Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
        )),
        Some(Value::Bool(value)) => Ok(Some(if *value { "1" } else { "0" }.to_owned())),
        Some(Value::Number(value)) => Ok(Some(value.to_string())),
    }
}
fn parse_card_timestamp(raw: &str) -> Option<String> {
    let parsed = chrono::DateTime::parse_from_rfc3339(raw)
        .map(|date| date.with_timezone(&Utc))
        .ok()
        .or_else(|| {
            let raw = raw.strip_suffix(" UTC").unwrap_or(raw);
            [
                "%Y-%m-%d %H:%M:%S%.f",
                "%Y-%m-%dT%H:%M:%S%.f",
                "%Y/%m/%d %H:%M:%S%.f",
                "%B %-d, %Y %H:%M",
            ]
            .iter()
            .find_map(|format| chrono::NaiveDateTime::parse_from_str(raw, format).ok())
            .map(|date| date.and_utc())
            .or_else(|| {
                chrono::NaiveDate::parse_from_str(raw, "%Y-%m-%d")
                    .ok()
                    .and_then(|date| date.and_hms_opt(0, 0, 0))
                    .map(|date| date.and_utc())
            })
        })?;
    Some(parsed.to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
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
fn signed_in_frame_page(account: i64, title: &str, frame: String) -> Response {
    signed_in_shell_html(account, title, &frame, "", "", "").into_response()
}
fn signed_in_shell_html(
    account: i64,
    title: &str,
    content: &str,
    extra_head: &str,
    extra_styles: &str,
    header_content: &str,
) -> Html<String> {
    let footer =
        include_str!("../static/home_footer.html").replace("{{ACCOUNT}}", &account.to_string());
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1, user-scalable=no, viewport-fit=cover\"><title>{title} | Rizzy</title>{extra_head}<style>:root{{color-scheme:dark;font:16px/1.5 system-ui,sans-serif;background:#101b20;color:#f4f5f6}}body{{margin:0}}#global-container{{min-height:100vh;padding-bottom:50px}}#main{{max-width:960px;margin:0 auto;padding:24px}}.header{{padding:12px 24px;border-bottom:1px solid #304049}}a{{color:#80b7fc}}{styles}{menu_styles}{extra_styles}</style></head><body data-account=\"{account}\"><div id=\"global-container\"><header class=\"header\" id=\"header\"><a class=\"header__skip-navigation\" href=\"#main\">Skip to main content</a><a href=\"/{account}/my/menu\" data-open-menu aria-haspopup=\"dialog\" aria-expanded=\"false\">Rizzy</a>{header_content}</header><dialog id=\"menu-dialog\" aria-label=\"Rizzy menu\"><turbo-frame id=\"my_menu\"></turbo-frame></dialog><main id=\"main\">{content}</main></div>{footer}<script>{script}</script></body></html>",
        title = escape(title),
        account = account,
        styles = include_str!("../static/shell_footer.css"),
        menu_styles = include_str!("../static/menu_overlay.css"),
        script = include_str!("../static/events.js"),
    ))
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
fn geared_page_count(total: i64) -> i64 {
    match total {
        ..=15 => 1,
        16..=45 => 2,
        46..=95 => 3,
        96..=195 => 4,
        _ => 5 + (total - 196) / 100,
    }
}
fn paginated(items: Value, total: i64, page: i64, uri: &Uri, headers: &HeaderMap) -> Response {
    pagination_headers(Json(items).into_response(), total, page, uri, headers)
}
fn card_page_etag(rows: &[DbRow]) -> String {
    let mut digest = Sha256::new();
    for row in rows {
        digest.update(row.get::<String, _>("id").as_bytes());
        digest.update([0]);
        digest.update(row.get::<String, _>("updated_at").as_bytes());
        digest.update([0]);
    }
    format!("W/\"{}\"", hex::encode(&digest.finalize()[..16]))
}
fn etag_matches(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|candidates| {
            candidates
                .split(',')
                .any(|candidate| candidate.trim() == etag || candidate.trim() == "*")
        })
}
fn cached_card_page(
    etag: &str,
    total: i64,
    page: i64,
    uri: &Uri,
    headers: &HeaderMap,
) -> Option<Response> {
    if !etag_matches(headers, etag) {
        return None;
    }
    Some(with_card_page_headers(
        pagination_headers(
            StatusCode::NOT_MODIFIED.into_response(),
            total,
            page,
            uri,
            headers,
        ),
        etag,
    ))
}
fn with_card_page_headers(mut response: Response, etag: &str) -> Response {
    response
        .headers_mut()
        .insert(header::ETAG, HeaderValue::from_str(etag).expect("hex ETag"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=0, private, must-revalidate"),
    );
    response
}
fn pagination_headers(
    mut response: Response,
    total: i64,
    page: i64,
    uri: &Uri,
    headers: &HeaderMap,
) -> Response {
    response.headers_mut().insert(
        "x-total-count",
        HeaderValue::from_str(&total.to_string()).expect("count header"),
    );
    // GearedPagination emits a next link on an out-of-range page because only
    // the exact last page suppresses it.
    if page != geared_page_count(total) {
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
    account_updated_at: String,
    user_id: String,
    role: String,
    permission: String,
    slug: i64,
    request_id: String,
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
    let r = sqlx::query("SELECT a.id account_id,a.updated_at account_updated_at,u.id user_id,u.role,EXISTS(SELECT 1 FROM account_cancellations c WHERE c.account_id=a.id) cancelled FROM users u JOIN accounts a ON a.id=u.account_id WHERE u.identity_id=? AND a.external_account_id=? AND u.active=1")
        .bind(identity_id).bind(slug).fetch_optional(&app.db).await.map_err(db_err)?
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED,"Invalid token or account"))?;
    if r.get::<i64, _>("cancelled") != 0 {
        return Err(if headers.contains_key("x-rizzy-json-request") {
            err(StatusCode::FORBIDDEN, "Account access required")
        } else {
            ApiError(StatusCode::FOUND, "/session/menu".into(), false)
        });
    }
    Ok(Principal {
        account_id: r.get("account_id"),
        account_updated_at: r.get("account_updated_at"),
        user_id: r.get("user_id"),
        role: r.get("role"),
        permission,
        slug,
        request_id: headers
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or_default()
            .to_owned(),
    })
}
async fn board(app: &App, p: &Principal, id: &str) -> Result<DbRow> {
    sqlx::query("SELECT b.* FROM boards b WHERE b.id=? AND b.account_id=? AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?)")
        .bind(id.trim_end_matches(".json")).bind(&p.account_id).bind(&p.user_id).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)
}
async fn card(app: &App, p: &Principal, number: &str) -> Result<DbRow> {
    let n: i64 = number
        .trim_end_matches(".json")
        .parse()
        .map_err(|_| missing())?;
    sqlx::query("SELECT c.* FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.account_id=? AND c.number=? AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?)")
        .bind(&p.account_id).bind(n).bind(&p.user_id).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)
}
async fn user_json(app: &App, p: &Principal, id: &str, host: &str) -> Result<Value> {
    let r = sqlx::query("SELECT u.*,i.email_address FROM users u LEFT JOIN identities i ON i.id=u.identity_id WHERE u.id=? AND u.account_id=?")
        .bind(id).bind(&p.account_id).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    Ok(user_json_row(p, &r, host))
}
fn user_json_row(p: &Principal, r: &DbRow, host: &str) -> Value {
    let id: String = r.get("id");
    let role: String = r.get("role");
    let email: Option<String> = if role == "system" {
        None
    } else {
        r.get::<Option<String>, _>("email_address")
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
    if app.mysql {
        let mut tx = app.db.begin().await.map_err(db_err)?;
        sqlx::query("SELECT id FROM accounts WHERE id=? FOR UPDATE")
            .bind(&p.account_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
        if let Some(existing) = sqlx::query_scalar::<_, String>(
            "SELECT id FROM users WHERE account_id=? AND role='system'",
        )
        .bind(&p.account_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?
        {
            tx.commit().await.map_err(db_err)?;
            return Ok(existing);
        }
        let user_id = id();
        sqlx::query(
            "INSERT INTO users(id,account_id,name,role,created_at) VALUES(?,?,'System','system',?)",
        )
        .bind(&user_id)
        .bind(&p.account_id)
        .bind(now())
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        tx.commit().await.map_err(db_err)?;
        return Ok(user_id);
    }
    let user_id = id();
    let timestamp = now();
    sqlx::query("INSERT OR IGNORE INTO users(id,account_id,name,role,created_at) VALUES(?,?,'System','system',?)")
        .bind(&user_id)
        .bind(&p.account_id)
        .bind(&timestamp)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
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
    let mut tx = begin_write(&app).await.map_err(db_err)?;
    insert_system_comment_tx(
        app, &mut tx, p, &system, card_id, body, &content, &timestamp,
    )
    .await?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}
async fn insert_system_comment_tx(
    app: &App,
    tx: &mut sqlx::Transaction<'_, Db>,
    p: &Principal,
    system: &str,
    card_id: &str,
    body: &str,
    content: &str,
    timestamp: &str,
) -> Result<()> {
    let comment_id = id();
    sqlx::query("INSERT INTO comments(id,card_id,creator_id,body,search_text,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
        .bind(&comment_id).bind(card_id).bind(system).bind(body).bind(content).bind(timestamp).bind(timestamp).execute(&mut **tx).await.map_err(db_err)?;
    rich_text_records::sync(
        tx,
        &p.account_id,
        "Comment",
        &comment_id,
        "body",
        "",
        body,
        timestamp,
    )
    .await?;
    if app.mysql {
        mysql_search::sync_comment(tx, &p.account_id, &comment_id)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}
async fn insert_event_tx(
    app: &App,
    tx: &mut sqlx::Transaction<'_, Db>,
    p: &Principal,
    board_id: &str,
    card_id: &str,
    eventable_type: &str,
    eventable_id: &str,
    action: &str,
    particulars: &Value,
    timestamp: &str,
) -> Result<()> {
    let event_id = id();
    sqlx::query("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,particulars,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
        .bind(&event_id).bind(&p.account_id).bind(board_id).bind(card_id).bind(&p.user_id).bind(eventable_type)
        .bind(eventable_id).bind(action).bind(particulars.to_string()).bind(timestamp).bind(timestamp)
        .execute(&mut **tx).await.map_err(db_err)?;
    queue_webhooks(tx, &p.account_id, board_id, &event_id, action, timestamp).await?;
    queue_notifications(
        app,
        tx,
        p,
        board_id,
        card_id,
        &event_id,
        eventable_type,
        action,
        particulars,
        timestamp,
    )
    .await?;
    sqlx::query("UPDATE cards SET last_active_at=?,updated_at=? WHERE id=?")
        .bind(timestamp)
        .bind(timestamp)
        .bind(card_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
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
    let (mut tx, event_board_id) = begin_card_board_write(app, card_id, board_id).await?;
    insert_event_tx(
        app,
        &mut tx,
        p,
        &event_board_id,
        card_id,
        eventable_type,
        eventable_id,
        action,
        &particulars,
        &timestamp,
    )
    .await?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}
async fn record_activity_spike(app: &App, account_id: &str, card_id: &str) -> Result<()> {
    let timestamp = now();
    let board_id: String = sqlx::query_scalar("SELECT board_id FROM cards WHERE id=?")
        .bind(card_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let (mut tx, _) = begin_card_board_write(app, card_id, &board_id).await?;
    let statement = if app.mysql {
        "INSERT INTO card_activity_spikes(id,card_id,account_id,created_at,updated_at) VALUES(?,?,?,?,?) ON DUPLICATE KEY UPDATE updated_at=VALUES(updated_at)"
    } else {
        "INSERT INTO card_activity_spikes(id,card_id,account_id,created_at,updated_at) VALUES(?,?,?,?,?) ON CONFLICT(card_id) DO UPDATE SET updated_at=excluded.updated_at"
    };
    sqlx::query(statement)
        .bind(uuid::Uuid::now_v7().to_string())
        .bind(card_id)
        .bind(account_id)
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("UPDATE cards SET updated_at=? WHERE id=?")
        .bind(&timestamp)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
}
async fn queue_webhooks(
    tx: &mut sqlx::Transaction<'_, Db>,
    account_id: &str,
    board_id: &str,
    event_id: &str,
    action: &str,
    timestamp: &str,
) -> Result<()> {
    let statement = if (**tx).backend_name() == "MySQL" {
        "SELECT w.id FROM webhooks w WHERE w.account_id=? AND w.board_id=? AND w.active=1 AND JSON_CONTAINS(w.subscribed_actions,JSON_QUOTE(?))"
    } else {
        "SELECT w.id FROM webhooks w WHERE w.account_id=? AND w.board_id=? AND w.active=1 AND EXISTS(SELECT 1 FROM json_each(w.subscribed_actions) WHERE value=?)"
    };
    let webhooks: Vec<String> = sqlx::query_scalar(statement)
        .bind(account_id)
        .bind(board_id)
        .bind(action)
        .fetch_all(&mut **tx)
        .await
        .map_err(db_err)?;
    for webhook_id in webhooks {
        sqlx::query("INSERT INTO webhook_deliveries(id,account_id,webhook_id,event_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(id()).bind(account_id).bind(webhook_id).bind(event_id).bind(timestamp).bind(timestamp)
            .execute(&mut **tx).await.map_err(db_err)?;
    }
    Ok(())
}

async fn referenced_users(
    app: &App,
    tx: &mut sqlx::Transaction<'_, Db>,
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
        } else if let Some(signed_id) = app.sgid_signer.parse_user(&sgid) {
            sqlx::query_scalar("SELECT id FROM users WHERE id=? AND account_id=?")
                .bind(signed_id)
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
    app: &App,
    tx: &mut sqlx::Transaction<'_, Db>,
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
        referenced_users(app, tx, &p.account_id, &content)
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
        let statement = if (**tx).backend_name() == "MySQL" {
            "INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,unread_count,created_at,updated_at) SELECT ?,?,?,?,?,?,'Event',1,?,? WHERE EXISTS(SELECT 1 FROM accesses a JOIN users u ON u.id=a.user_id WHERE a.board_id=? AND a.user_id=? AND u.active=1) ON DUPLICATE KEY UPDATE source_id=VALUES(source_id),creator_id=VALUES(creator_id),source_type='Event',read_at=NULL,unread_count=unread_count+1,updated_at=VALUES(updated_at)"
        } else {
            "INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,unread_count,created_at,updated_at) SELECT ?,?,?,?,?,?,'Event',1,?,? WHERE EXISTS(SELECT 1 FROM accesses a JOIN users u ON u.id=a.user_id WHERE a.board_id=? AND a.user_id=? AND u.active=1) ON CONFLICT(user_id,card_id) DO UPDATE SET source_id=excluded.source_id,creator_id=excluded.creator_id,source_type='Event',read_at=NULL,unread_count=notifications.unread_count+1,updated_at=excluded.updated_at"
        };
        let inserted = sqlx::query(statement)
            .bind(id())
            .bind(&p.account_id)
            .bind(&recipient)
            .bind(&p.user_id)
            .bind(card_id)
            .bind(event_id)
            .bind(timestamp)
            .bind(timestamp)
            .bind(board_id)
            .bind(&recipient)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        if inserted.rows_affected() != 0 {
            notification_mail::queue(tx, &recipient, timestamp).await?;
            push_delivery::queue(tx, &recipient, card_id).await?;
        }
    }
    Ok(())
}

async fn create_new_mentions(
    app: &App,
    tx: &mut sqlx::Transaction<'_, Db>,
    p: &Principal,
    board_id: &str,
    card_id: &str,
    source_type: &str,
    source_id: &str,
    content: &str,
) -> Result<()> {
    for user_id in referenced_users(app, tx, &p.account_id, content).await? {
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
        let watch_statement = if (**tx).backend_name() == "MySQL" {
            "INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,1,?,?) ON DUPLICATE KEY UPDATE updated_at=IF(watching<>1,VALUES(updated_at),updated_at),watching=1"
        } else {
            "INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,1,?,?) ON CONFLICT(card_id,user_id) DO UPDATE SET watching=1,updated_at=excluded.updated_at WHERE watches.watching!=1"
        };
        sqlx::query(watch_statement)
            .bind(id())
            .bind(card_id)
            .bind(&user_id)
            .bind(&timestamp)
            .bind(&timestamp)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        if user_id != p.user_id && p.role != "system" {
            let statement = if (**tx).backend_name() == "MySQL" {
                "INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,unread_count,created_at,updated_at) VALUES(?,?,?,?,?,?,'Mention',1,?,?) ON DUPLICATE KEY UPDATE source_id=VALUES(source_id),creator_id=VALUES(creator_id),source_type='Mention',read_at=NULL,unread_count=unread_count+1,updated_at=VALUES(updated_at)"
            } else {
                "INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,source_type,unread_count,created_at,updated_at) VALUES(?,?,?,?,?,?,'Mention',1,?,?) ON CONFLICT(user_id,card_id) DO UPDATE SET source_id=excluded.source_id,creator_id=excluded.creator_id,source_type='Mention',read_at=NULL,unread_count=notifications.unread_count+1,updated_at=excluded.updated_at"
            };
            sqlx::query(statement)
                .bind(id())
                .bind(&p.account_id)
                .bind(&user_id)
                .bind(&p.user_id)
                .bind(card_id)
                .bind(&mention_id)
                .bind(&timestamp)
                .bind(&timestamp)
                .execute(&mut **tx)
                .await
                .map_err(db_err)?;
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

fn first_public_address(addresses: impl IntoIterator<Item = SocketAddr>) -> Option<SocketAddr> {
    addresses
        .into_iter()
        .find(|address| public_ip(address.ip()))
}

const MAX_WEBHOOK_RESPONSE_SIZE: usize = 100 * 1024;

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
    use super::{first_public_address, hmac_sha256, public_ip};

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

    #[test]
    fn mixed_dns_answers_pin_the_first_public_address() {
        let addresses = [
            "10.0.0.1:443".parse().unwrap(),
            "93.184.216.34:443".parse().unwrap(),
            "192.168.1.1:443".parse().unwrap(),
            "1.1.1.1:443".parse().unwrap(),
        ];
        assert_eq!(first_public_address(addresses), Some(addresses[1]));
        assert_eq!(first_public_address([addresses[0], addresses[2]]), None);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WebhookFormat {
    Generic,
    Slack,
    Campfire,
    Basecamp,
}

fn webhook_format(url: &str) -> WebhookFormat {
    static SLACK: OnceLock<Regex> = OnceLock::new();
    static CAMPFIRE: OnceLock<Regex> = OnceLock::new();
    static BASECAMP: OnceLock<Regex> = OnceLock::new();
    let pattern = |pattern| {
        RegexBuilder::new(pattern)
            .case_insensitive(true)
            .build()
            .unwrap()
    };
    if BASECAMP
        .get_or_init(|| pattern(r"/\d+/integrations/[^/]+/buckets/\d+/chats/\d+/lines\z"))
        .is_match(url)
    {
        WebhookFormat::Basecamp
    } else if CAMPFIRE
        .get_or_init(|| pattern(r"/rooms/\d+/\d+-[^/]+/messages\z"))
        .is_match(url)
    {
        WebhookFormat::Campfire
    } else if SLACK
        .get_or_init(|| pattern(r"//hooks\.slack\.com/services/T[^/]+/B[^/]+/[^/]+\z"))
        .is_match(url)
    {
        WebhookFormat::Slack
    } else {
        WebhookFormat::Generic
    }
}

impl WebhookFormat {
    fn content_type(self) -> &'static str {
        match self {
            Self::Campfire => "text/html",
            Self::Basecamp => "application/x-www-form-urlencoded",
            Self::Generic | Self::Slack => "application/json",
        }
    }

    fn body(self, item: &Value) -> String {
        match self {
            Self::Generic => webhook_generic_json(item),
            Self::Slack => webhook_wire_json(json!({"text":format!("{} <{}|Open in Fizzy>",
                item["description"].as_str().unwrap_or(""),
                item["url"].as_str().unwrap_or(""))})),
            Self::Campfire | Self::Basecamp => {
                let html = format!(
                    "{}\n<a href=\"{}\">↗︎</a>",
                    item["description"].as_str().unwrap_or(""),
                    escape(item["url"].as_str().unwrap_or(""))
                );
                if self == Self::Basecamp {
                    url::form_urlencoded::Serializer::new(String::new())
                        .append_pair("content", &html)
                        .finish()
                } else {
                    html
                }
            }
        }
    }
}

fn webhook_wire_json(value: Value) -> String {
    webhook_escape_json(value.to_string())
}

fn webhook_escape_json(value: String) -> String {
    value
        .replace('&', "\\u0026")
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

#[derive(Clone, Copy)]
enum WebhookJsonObject {
    Event,
    Card,
    Comment,
    Board,
    Column,
    User,
    Body,
    CardRef,
}

fn webhook_generic_json(event: &Value) -> String {
    webhook_escape_json(webhook_ordered_json(event, WebhookJsonObject::Event))
}

fn webhook_ordered_json(value: &Value, kind: WebhookJsonObject) -> String {
    use WebhookJsonObject as Object;
    let fields: &[&str] = match kind {
        Object::Event => &[
            "id",
            "action",
            "created_at",
            "eventable",
            "board",
            "creator",
        ],
        Object::Card => &[
            "id",
            "number",
            "title",
            "status",
            "description",
            "description_html",
            "image_url",
            "has_attachments",
            "tags",
            "closed",
            "postponed",
            "golden",
            "last_active_at",
            "created_at",
            "url",
            "board",
            "column",
            "creator",
            "assignees",
            "has_more_assignees",
            "comments_url",
            "reactions_url",
        ],
        Object::Comment => &[
            "id",
            "created_at",
            "updated_at",
            "body",
            "creator",
            "card",
            "reactions_url",
            "url",
        ],
        Object::Board => &[
            "id",
            "name",
            "all_access",
            "created_at",
            "auto_postpone_period_in_days",
            "url",
            "creator",
            "public_description",
            "public_description_html",
            "public_url",
        ],
        Object::Column => &["id", "name", "color", "created_at", "cards_url"],
        Object::User => &[
            "id",
            "name",
            "role",
            "active",
            "email_address",
            "created_at",
            "url",
            "avatar_url",
        ],
        Object::Body => &["plain_text", "html"],
        Object::CardRef => &["id", "url"],
    };
    let mut output = String::from("{");
    for key in fields {
        let Some(child) = value.get(*key) else {
            continue;
        };
        if output.len() > 1 {
            output.push(',');
        }
        output.push_str(&serde_json::to_string(key).expect("static JSON key"));
        output.push(':');
        let nested = match *key {
            "eventable" if child.get("number").is_some() => Some(Object::Card),
            "eventable" if child.get("body").is_some() => Some(Object::Comment),
            "board" => Some(Object::Board),
            "column" => Some(Object::Column),
            "creator" => Some(Object::User),
            "body" => Some(Object::Body),
            "card" => Some(Object::CardRef),
            _ => None,
        };
        if let Some(nested) = nested {
            output.push_str(&webhook_ordered_json(child, nested));
        } else if *key == "assignees" {
            output.push('[');
            for (index, assignee) in child.as_array().into_iter().flatten().enumerate() {
                if index > 0 {
                    output.push(',');
                }
                output.push_str(&webhook_ordered_json(assignee, Object::User));
            }
            output.push(']');
        } else {
            output.push_str(&serde_json::to_string(child).expect("JSON value"));
        }
    }
    output.push('}');
    output
}

#[cfg(test)]
mod webhook_format_tests {
    use super::{WebhookFormat, webhook_format};
    use serde_json::{Value, json};

    fn event() -> Value {
        json!({"id":"event-1","action":"card_published","created_at":"2026-01-01T00:00:00Z",
            "description":"David added &quot;Tom &amp; Jerry&#39;s &lt;Great&gt; &quot;Adventure&quot;&quot;",
            "url":"http://example.org/897362094/cards/1",
            "eventable":{"title":"Tom & Jerry's <Great> \"Adventure\"",
                "url":"http://example.org/897362094/cards/1"},
            "board":{"id":"board-1"},"creator":{"name":"David"}})
    }

    #[test]
    fn specialized_payloads_match_fizzy_wire_formats() {
        let event = event();
        let html = "David added &quot;Tom &amp; Jerry&#39;s &lt;Great&gt; &quot;Adventure&quot;&quot;\n<a href=\"http://example.org/897362094/cards/1\">↗︎</a>";
        let basecamp = webhook_format(
            "https://3.basecamp.com/123/integrations/webhook/buckets/456/chats/789/lines",
        );
        assert_eq!(basecamp, WebhookFormat::Basecamp);
        assert_eq!(basecamp.content_type(), "application/x-www-form-urlencoded");
        assert_eq!(
            url::form_urlencoded::parse(basecamp.body(&event).as_bytes())
                .find(|(key, _)| key == "content")
                .unwrap()
                .1,
            html
        );

        let campfire = webhook_format("https://example.com/rooms/123/456-room-name/messages");
        assert_eq!(campfire, WebhookFormat::Campfire);
        assert_eq!(campfire.content_type(), "text/html");
        assert_eq!(campfire.body(&event), html);

        let slack = webhook_format("https://hooks.slack.com/services/T123/B456/secret");
        assert_eq!(slack, WebhookFormat::Slack);
        assert_eq!(slack.content_type(), "application/json");
        assert!(slack.body(&event).contains("\\u0026quot;"));
        assert_eq!(
            serde_json::from_str::<Value>(&slack.body(&event)).unwrap()["text"],
            "David added &quot;Tom &amp; Jerry&#39;s &lt;Great&gt; &quot;Adventure&quot;&quot; <http://example.org/897362094/cards/1|Open in Fizzy>"
        );

        let generic = webhook_format("https://example.com/webhook");
        assert_eq!(generic, WebhookFormat::Generic);
        let body: Value = serde_json::from_str(&generic.body(&event)).unwrap();
        assert_eq!(body["eventable"]["title"], event["eventable"]["title"]);
        assert!(body.get("text").is_none());

        let mut comment = event;
        comment["url"] = json!("http://example.org/897362094/cards/1#comment_abc");
        comment["eventable"]["url"] = json!("http://example.org/897362094/cards/1/comments/abc");
        assert!(slack.body(&comment).contains("#comment_abc|Open in Fizzy"));
        assert!(campfire.body(&comment).contains("#comment_abc\""));
    }
}

async fn webhook_payload(app: &App, row: &DbRow, host: &str) -> Result<(String, WebhookFormat)> {
    let account_id: String = row.get("account_id");
    let slug: i64 = row.get("external_account_id");
    let creator_id: String = row.get("creator_id");
    let principal = Principal {
        account_id,
        account_updated_at: row.get("account_updated_at"),
        user_id: creator_id,
        slug,
        role: "owner".into(),
        permission: "write".into(),
        request_id: String::new(),
    };
    let event = sqlx::query("SELECT * FROM events WHERE id=?")
        .bind(row.get::<String, _>("event_id"))
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let item = activity_items(app, &principal, &[event], host, None)
        .await?
        .into_iter()
        .next()
        .ok_or_else(missing)?;
    let format = webhook_format(&row.get::<String, _>("url"));
    Ok((format.body(&item), format))
}

async fn send_webhook(
    url: &str,
    secret: &str,
    timestamp: &str,
    body: &str,
    content_type: &str,
) -> Value {
    let Ok(uri) = url::Url::parse(url) else {
        return json!({"error":"destination_unreachable"});
    };
    if !matches!(uri.scheme(), "http" | "https") {
        return json!({"error":"destination_unreachable"});
    }
    let Some(host) = uri.host_str().map(str::to_owned) else {
        return json!({"error":"dns_lookup_failed"});
    };
    let Some(port) = uri.port_or_known_default() else {
        return json!({"error":"destination_unreachable"});
    };
    let addresses = if let Ok(ip) = host.parse::<IpAddr>() {
        vec![SocketAddr::new(ip, port)]
    } else {
        match tokio::time::timeout(Duration::from_secs(7), lookup_host((host.as_str(), port))).await
        {
            Ok(Ok(found)) => found.collect::<Vec<_>>(),
            _ => return json!({"error":"dns_lookup_failed"}),
        }
    };
    if addresses.is_empty() {
        return json!({"error":"dns_lookup_failed"});
    }
    let Some(address) = first_public_address(addresses) else {
        return json!({"error":"private_uri"});
    };
    send_webhook_pinned(uri, &host, address, secret, timestamp, body, content_type).await
}

fn webhook_request_error(error: &reqwest::Error) -> Value {
    if error.is_timeout() {
        return json!({"error":"connection_timeout"});
    }
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = cause {
        if current.downcast_ref::<rustls::Error>().is_some() {
            return json!({"error":"failed_tls"});
        }
        cause = current
            .downcast_ref::<std::io::Error>()
            .and_then(std::io::Error::get_ref)
            .map(|inner| inner as &(dyn std::error::Error + 'static))
            .or_else(|| current.source());
    }
    json!({"error":"destination_unreachable"})
}

async fn send_webhook_pinned(
    uri: url::Url,
    host: &str,
    address: SocketAddr,
    secret: &str,
    timestamp: &str,
    body: &str,
    content_type: &str,
) -> Value {
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(7))
        .redirect(reqwest::redirect::Policy::none())
        .no_proxy()
        .resolve_to_addrs(host, &[address])
        .build()
    {
        Ok(client) => client,
        Err(_) => return json!({"error":"destination_unreachable"}),
    };
    let result = client
        .post(uri)
        .header("User-Agent", "fizzy/1.0.0 Webhook")
        .header("Content-Type", content_type)
        .header("X-Webhook-Timestamp", timestamp)
        .header("X-Webhook-Signature", hmac_sha256(secret, body))
        .body(body.to_owned())
        .send()
        .await;
    let Ok(mut response) = result else {
        return webhook_request_error(&result.unwrap_err());
    };
    let code = response.status().as_u16();
    let mut bytes = 0usize;
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                bytes += chunk.len();
                if bytes > MAX_WEBHOOK_RESPONSE_SIZE {
                    return json!({"error":"response_too_large"});
                }
            }
            Ok(None) => break,
            Err(error) => return webhook_request_error(&error),
        }
    }
    json!({"code":code})
}

#[cfg(test)]
mod webhook_transport_tests {
    use super::{MAX_WEBHOOK_RESPONSE_SIZE, hmac_sha256, send_webhook_pinned};
    use serde_json::json;
    use std::net::SocketAddr;
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
    };

    async fn serve_http(body_size: usize) -> (SocketAddr, tokio::task::JoinHandle<String>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 4096];
            let header_end = loop {
                let count = stream.read(&mut chunk).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&chunk[..count]);
                if let Some(end) = request.windows(4).position(|bytes| bytes == b"\r\n\r\n") {
                    break end + 4;
                }
            };
            let headers = String::from_utf8_lossy(&request[..header_end]).into_owned();
            let content_length: usize = headers
                .lines()
                .find_map(|line| {
                    line.to_ascii_lowercase()
                        .strip_prefix("content-length: ")
                        .and_then(|value| value.trim().parse().ok())
                })
                .unwrap();
            while request.len() - header_end < content_length {
                let count = stream.read(&mut chunk).await.unwrap();
                assert!(count > 0);
                request.extend_from_slice(&chunk[..count]);
            }
            assert_eq!(&request[header_end..], b"wire payload");
            stream.write_all(format!("HTTP/1.1 202 Accepted\r\nContent-Length: {body_size}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
            stream.write_all(&vec![b'x'; body_size]).await.unwrap();
            headers
        });
        (address, handle)
    }

    #[tokio::test]
    async fn pinned_delivery_signs_body_and_enforces_fizzy_size_boundary() {
        for (body_size, expected) in [
            (MAX_WEBHOOK_RESPONSE_SIZE, json!({"code":202})),
            (
                MAX_WEBHOOK_RESPONSE_SIZE + 1,
                json!({"error":"response_too_large"}),
            ),
        ] {
            let (address, server) = serve_http(body_size).await;
            let actual = send_webhook_pinned(
                "http://webhook.example/receive".parse().unwrap(),
                "webhook.example",
                address,
                "shared secret",
                "2026-01-01T00:00:00Z",
                "wire payload",
                "application/json",
            )
            .await;
            assert_eq!(actual, expected);
            let request = server.await.unwrap();
            assert!(request.contains("content-type: application/json\r\n"));
            assert!(request.contains("x-webhook-timestamp: 2026-01-01T00:00:00Z\r\n"));
            assert!(request.contains(&format!(
                "x-webhook-signature: {}\r\n",
                hmac_sha256("shared secret", "wire payload")
            )));
        }
    }

    #[tokio::test]
    async fn invalid_tls_response_is_reported_as_failed_tls() {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut hello = [0u8; 4096];
            assert!(stream.read(&mut hello).await.unwrap() > 0);
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")
                .await
                .unwrap();
        });
        let result = send_webhook_pinned(
            "https://webhook.example/receive".parse().unwrap(),
            "webhook.example",
            address,
            "shared secret",
            "2026-01-01T00:00:00Z",
            "wire payload",
            "application/json",
        )
        .await;
        server.await.unwrap();
        assert_eq!(result, json!({"error":"failed_tls"}));
    }
}

async fn process_webhook_delivery(app: &App, delivery_id: &str, host: &str) -> Result<()> {
    let row = sqlx::query("SELECT d.*,w.url,w.signing_secret,e.creator_id,e.created_at event_created_at,a.external_account_id,a.updated_at account_updated_at FROM webhook_deliveries d JOIN webhooks w ON w.id=d.webhook_id JOIN events e ON e.id=d.event_id JOIN accounts a ON a.id=d.account_id WHERE d.id=?")
        .bind(delivery_id).fetch_one(&app.db).await.map_err(db_err)?;
    let timestamp: String = row.get("event_created_at");
    let timestamp = chrono::DateTime::parse_from_rfc3339(&timestamp)
        .map(|t| {
            t.to_utc()
                .to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
        })
        .unwrap_or(timestamp);
    let (payload, format) = webhook_payload(app, &row, host).await?;
    let request = json!({"headers":{"User-Agent":"fizzy/1.0.0 Webhook","Content-Type":format.content_type(),"X-Webhook-Timestamp":timestamp}});
    let response = send_webhook(
        &row.get::<String, _>("url"),
        &row.get::<String, _>("signing_secret"),
        &timestamp,
        &payload,
        format.content_type(),
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
            "UPDATE webhook_delinquency_trackers SET consecutive_failures_count=0,first_failure_at=NULL WHERE webhook_id=?",
        )
        .bind(&webhook_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    } else {
        sqlx::query("UPDATE webhook_delinquency_trackers SET first_failure_at=CASE WHEN consecutive_failures_count=0 THEN ? ELSE first_failure_at END,consecutive_failures_count=consecutive_failures_count+1,updated_at=? WHERE webhook_id=?")
            .bind(&completed_at).bind(&completed_at).bind(&webhook_id)
            .execute(&mut *tx).await.map_err(db_err)?;
        let tracker = sqlx::query("SELECT consecutive_failures_count,first_failure_at FROM webhook_delinquency_trackers WHERE webhook_id=?")
            .bind(&webhook_id).fetch_one(&mut *tx).await.map_err(db_err)?;
        let first_failure: Option<String> = tracker.get("first_failure_at");
        let delinquent = tracker.get::<i64, _>("consecutive_failures_count") >= 10
            && first_failure
                .as_deref()
                .and_then(|value| chrono::DateTime::parse_from_rfc3339(value).ok())
                .is_some_and(|first| first < Utc::now() - chrono::Duration::hours(1));
        if delinquent {
            sqlx::query("UPDATE webhooks SET active=0,updated_at=? WHERE id=?")
                .bind(&completed_at)
                .bind(&webhook_id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
    }
    tx.commit().await.map_err(db_err)?;
    Ok(())
}

async fn claim_webhook_delivery(app: &App) -> sqlx::Result<Option<String>> {
    if app.mysql {
        let mut tx = app.db.begin().await?;
        let delivery_id: Option<String> = sqlx::query_scalar(
            "SELECT id FROM webhook_deliveries WHERE state='pending' ORDER BY created_at,id LIMIT 1 FOR UPDATE SKIP LOCKED",
        )
        .fetch_optional(&mut *tx)
        .await?;
        if let Some(delivery_id) = delivery_id {
            let updated = sqlx::query("UPDATE webhook_deliveries SET state='in_progress',updated_at=? WHERE id=? AND state='pending'")
                .bind(now())
                .bind(&delivery_id)
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
            return Ok((updated.rows_affected() == 1).then_some(delivery_id));
        }
        tx.commit().await?;
        Ok(None)
    } else {
        sqlx::query_scalar("UPDATE webhook_deliveries SET state='in_progress',updated_at=? WHERE id=(SELECT id FROM webhook_deliveries WHERE state='pending' ORDER BY created_at,id LIMIT 1) AND state='pending' RETURNING id")
            .bind(now())
            .fetch_optional(&app.db)
            .await
    }
}

async fn webhook_worker(app: Arc<App>) {
    let host = env::var("RIZZY_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3007".into());
    let semaphore = Arc::new(tokio::sync::Semaphore::new(8));
    let mut recovered_at = tokio::time::Instant::now() - Duration::from_secs(60);
    loop {
        if recovered_at.elapsed() >= Duration::from_secs(60) {
            let stale = (Utc::now() - chrono::Duration::minutes(15))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            if let Err(error) = sqlx::query("UPDATE webhook_deliveries SET state='pending' WHERE state='in_progress' AND updated_at<?")
                .bind(stale).execute(&app.db).await { eprintln!("webhook recovery failed: {error}"); }
            recovered_at = tokio::time::Instant::now();
        }
        let permit = semaphore
            .clone()
            .acquire_owned()
            .await
            .expect("open semaphore");
        match claim_webhook_delivery(&app).await {
            Ok(Some(delivery_id)) => {
                let app = app.clone();
                let host = host.clone();
                tokio::spawn(async move {
                    let _permit = permit;
                    if let Err(error) = process_webhook_delivery(&app, &delivery_id, &host).await {
                        eprintln!("webhook delivery {delivery_id} failed: {}", error.1);
                        let _ = sqlx::query("UPDATE webhook_deliveries SET state='errored',response=?,updated_at=? WHERE id=?")
                            .bind(json!({"error":"delivery_failed"}).to_string()).bind(now()).bind(&delivery_id)
                            .execute(&app.db).await;
                    }
                });
            }
            Ok(None) => {
                drop(permit);
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            Err(error) => {
                drop(permit);
                eprintln!("webhook claim failed: {error}");
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
        }
    }
}
async fn board_json(app: &App, p: &Principal, r: &DbRow, host: &str) -> Result<Value> {
    let creator: String = r.get("creator_id");
    let id: String = r.get("id");
    let present = if r.get::<Option<String>, _>("publication_key").is_some()
        && r.get::<String, _>("public_description").is_empty()
    {
        !rich_text_records::present_ids(&app.db, "Board", "description", &[id])
            .await?
            .is_empty()
    } else {
        true
    };
    Ok(board_json_row(
        p,
        r,
        host,
        user_json(app, p, &creator, host).await?,
        present,
    ))
}
fn board_json_row(
    p: &Principal,
    r: &DbRow,
    host: &str,
    creator: Value,
    description_present: bool,
) -> Value {
    let id: String = r.get("id");
    let all = r.get::<i64, _>("all_access") != 0;
    let mut value = json!({"id":id,"name":r.get::<String,_>("name"),"all_access":all,"created_at":r.get::<String,_>("created_at"),"auto_postpone_period_in_days":r.get::<i64,_>("auto_postpone_period_in_days"),"url":format!("{host}/{}/boards/{id}",p.slug),"creator":creator});
    if let Some(key) = r.get::<Option<String>, _>("publication_key") {
        let description: String = r.get("public_description");
        value["public_description"] = json!(rich_text::plain_text(&description));
        value["public_description_html"] =
            json!(if description.is_empty() && !description_present {
                String::new()
            } else {
                rich_html(&description)
            });
        value["public_url"] = json!(format!("{host}/{}/public/boards/{key}", p.slug));
    }
    value
}
async fn boards_json_batch(
    app: &App,
    p: &Principal,
    boards: &[DbRow],
    host: &str,
) -> Result<Vec<Value>> {
    if boards.is_empty() {
        return Ok(Vec::new());
    }
    let creator_ids = unique_ids(
        boards
            .iter()
            .map(|board| board.get::<String, _>("creator_id")),
    );
    let creators = rows_for_ids(
        app,
        "SELECT u.*,i.email_address FROM users u LEFT JOIN identities i ON i.id=u.identity_id WHERE u.id IN (",
        &creator_ids,
    )
    .await?;
    let users: HashMap<String, Value> = creators
        .iter()
        .filter(|user| user.get::<String, _>("account_id") == p.account_id)
        .map(|user| (user.get("id"), user_json_row(p, user, host)))
        .collect();
    let empty_description_ids = unique_ids(
        boards
            .iter()
            .filter(|board| {
                board.get::<Option<String>, _>("publication_key").is_some()
                    && board.get::<String, _>("public_description").is_empty()
            })
            .map(|board| board.get::<String, _>("id")),
    );
    let present_descriptions =
        rich_text_records::present_ids(&app.db, "Board", "description", &empty_description_ids)
            .await?;
    boards
        .iter()
        .map(|board| {
            let creator_id: String = board.get("creator_id");
            let creator = users.get(&creator_id).cloned().ok_or_else(missing)?;
            let board_id: String = board.get("id");
            Ok(board_json_row(
                p,
                board,
                host,
                creator,
                present_descriptions.contains(&board_id),
            ))
        })
        .collect()
}
async fn board_detail(app: &App, p: &Principal, r: &DbRow, host: &str) -> Result<Value> {
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
fn column_json(p: &Principal, r: &DbRow, host: &str) -> Value {
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
    r: &DbRow,
    host: &str,
    with_steps: bool,
) -> Result<Value> {
    let id: String = r.get("id");
    let board_id: String = r.get("board_id");
    let creator: String = r.get("creator_id");
    let description: String = r.get("description");
    let (description_plain, mut description_html, rendered_attachments) =
        storage::render_rich_text(app, &p.account_id, p.slug, &description).await?;
    let has_attachments = rendered_attachments
        || storage::embed_owners_for_cards(app, std::slice::from_ref(&id))
            .await?
            .contains(&id);
    if description.is_empty()
        && rich_text_records::present_ids(&app.db, "Card", "description", std::slice::from_ref(&id))
            .await?
            .is_empty()
    {
        description_html.clear();
    }
    let number: i64 = r.get("number");
    let tags: Vec<String> = sqlx::query_scalar("SELECT t.title FROM tags t JOIN taggings g ON g.tag_id=t.id WHERE g.card_id=? ORDER BY t.title").bind(&id).fetch_all(&app.db).await.map_err(db_err)?;
    let b = board(app, p, &board_id).await?;
    let created_at = r.get::<String, _>("created_at");
    let last_active_at = r.get::<String, _>("last_active_at");
    if parse_card_timestamp(&created_at).is_none()
        || parse_card_timestamp(&last_active_at).is_none()
    {
        return Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
        ));
    }
    let created_at = json_millisecond_time(created_at);
    let last_active_at = json_millisecond_time(last_active_at);
    let mut out = json!({"id":id,"number":number,"title":r.get::<String,_>("title"),"status":r.get::<String,_>("status"),"description":description_plain,"description_html":description_html,"image_url":null,"has_attachments":has_attachments,"tags":tags,"closed":r.get::<i64,_>("closed")!=0,"postponed":r.get::<i64,_>("postponed")!=0,"golden":r.get::<i64,_>("golden")!=0,"last_active_at":last_active_at,"created_at":created_at,"url":format!("{host}/{}/cards/{number}",p.slug),"board":board_json(app,p,&b,host).await?,"creator":user_json(app,p,&creator,host).await?,"assignees":[],"has_more_assignees":false,"comments_url":format!("{host}/{}/cards/{number}/comments",p.slug),"reactions_url":format!("{host}/{}/cards/{number}/reactions",p.slug)});
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
    cards: &[DbRow],
    host: &str,
) -> Result<Vec<Value>> {
    if cards.is_empty() {
        return Ok(Vec::new());
    }
    if cards.iter().any(|card| {
        parse_card_timestamp(&card.get::<String, _>("created_at")).is_none()
            || parse_card_timestamp(&card.get::<String, _>("last_active_at")).is_none()
    }) {
        return Err(err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal Server Error",
        ));
    }
    let card_ids = unique_ids(cards.iter().map(|c| c.get::<String, _>("id")));
    let empty_description_ids = unique_ids(
        cards
            .iter()
            .filter(|c| c.get::<String, _>("description").is_empty())
            .map(|c| c.get::<String, _>("id")),
    );
    let present_descriptions =
        rich_text_records::present_ids(&app.db, "Card", "description", &empty_description_ids)
            .await?;
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
    let empty_board_description_ids = unique_ids(
        boards
            .iter()
            .filter(|b| {
                b.get::<Option<String>, _>("publication_key").is_some()
                    && b.get::<String, _>("public_description").is_empty()
            })
            .map(|b| b.get::<String, _>("id")),
    );
    let present_board_descriptions = rich_text_records::present_ids(
        &app.db,
        "Board",
        "description",
        &empty_board_description_ids,
    )
    .await?;
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
    let users=rows_for_ids(app,"SELECT u.*,i.email_address FROM users u LEFT JOIN identities i ON i.id=u.identity_id WHERE u.id IN (",&user_ids).await?;
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
            board_json_row(
                p,
                board,
                host,
                creator,
                present_board_descriptions.contains(&board.get::<String, _>("id")),
            ),
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
    let embed_owners = storage::embed_owners_for_cards(app, &card_ids).await?;
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
        let (description_plain, mut description_html, rendered_attachments) =
            storage::render_rich_text(app, &p.account_id, p.slug, &description).await?;
        let has_attachments = rendered_attachments || embed_owners.contains(&card_id);
        if description.is_empty() && !present_descriptions.contains(&card_id) {
            description_html.clear();
        }
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
    sqlx::any::install_default_drivers();
    let url =
        env::var("RIZZY_DATABASE_URL").unwrap_or_else(|_| "sqlite://data/rizzy.sqlite3".into());
    let mysql = url.starts_with("mysql:");
    if !mysql && !url.starts_with("sqlite:") {
        return Err("RIZZY_DATABASE_URL must use sqlite: or mysql:".into());
    }
    let (db, qr_secret, board_etag_connection) = if mysql {
        let db = mysql_setup(&url).await?;
        let qr_secret = sqlx::query_scalar("SELECT value FROM app_secrets WHERE name='qr_codes'")
            .fetch_one(&db)
            .await?;
        (db, qr_secret, None)
    } else {
        if url.starts_with("sqlite://data/") {
            std::fs::create_dir_all("data")?;
        }
        let options = url
            .parse::<SqliteConnectOptions>()?
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .foreign_keys(true)
            .busy_timeout(Duration::from_secs(5));
        let pool_size = database_pool_size();
        let setup_connection = SqliteConnection::connect_with(&options).await?;
        drop(setup_connection);
        let db = AnyPoolOptions::new()
            .max_connections(if url.contains(":memory:") {
                1
            } else {
                pool_size
            })
            .after_connect(|connection, _| {
                Box::pin(async move {
                    sqlx::query("PRAGMA foreign_keys=ON")
                        .execute(&mut *connection)
                        .await?;
                    sqlx::query("PRAGMA busy_timeout=5000")
                        .execute(&mut *connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&url)
            .await?;
        sqlx::raw_sql(include_str!("../schema.sql"))
            .execute(&db)
            .await?;
        account_sequence::initialize(&db, false).await?;
        let bulk_touch_epoch_exists: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pragma_table_info('cards') WHERE name='bulk_touch_epoch'",
        )
        .fetch_one(&db)
        .await?;
        if bulk_touch_epoch_exists == 0 {
            sqlx::query("ALTER TABLE cards ADD COLUMN bulk_touch_epoch INTEGER NOT NULL DEFAULT 0")
                .execute(&db)
                .await?;
        }
        rich_text_records::migrate(&db).await?;
        for (name, definition) in [
            (
                "cards_touch_board_insert",
                "CREATE TRIGGER cards_touch_board_insert AFTER INSERT ON cards WHEN NEW.status='published' BEGIN UPDATE boards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.board_id; END",
            ),
            (
                "cards_touch_board_update",
                "CREATE TRIGGER cards_touch_board_update AFTER UPDATE ON cards WHEN NEW.bulk_touch_epoch=OLD.bulk_touch_epoch AND NEW.status='published' AND NOT EXISTS(SELECT 1 FROM card_bulk_touches WHERE board_id=NEW.board_id) BEGIN UPDATE boards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.board_id; END",
            ),
        ] {
            let existing: Option<String> =
                sqlx::query_scalar("SELECT sql FROM sqlite_master WHERE type='trigger' AND name=?")
                    .bind(name)
                    .fetch_optional(&db)
                    .await?;
            let current = existing.as_deref().is_some_and(|sql| {
                sql.contains("NEW.status='published'")
                    && (name != "cards_touch_board_update"
                        || (sql.contains("card_bulk_touches")
                            && sql.contains("NEW.bulk_touch_epoch=OLD.bulk_touch_epoch")))
            });
            if !current {
                let mut tx = db.begin().await?;
                sqlx::query(&format!("DROP TRIGGER IF EXISTS {name}"))
                    .execute(&mut *tx)
                    .await?;
                sqlx::query(definition).execute(&mut *tx).await?;
                tx.commit().await?;
            }
        }
        sqlx::query("CREATE TRIGGER IF NOT EXISTS cards_touch_column_insert AFTER INSERT ON cards WHEN NEW.column_id IS NOT NULL BEGIN UPDATE columns SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.column_id; END")
            .execute(&db).await?;
        let column_update_trigger: Option<String> = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='cards_touch_column_update'",
        )
        .fetch_optional(&db)
        .await?;
        if column_update_trigger.as_deref().is_none_or(|sql| {
            !sql.contains("card_bulk_touches")
                || !sql.contains("NEW.bulk_touch_epoch=OLD.bulk_touch_epoch")
        }) {
            let mut tx = db.begin().await?;
            sqlx::query("DROP TRIGGER IF EXISTS cards_touch_column_update")
                .execute(&mut *tx)
                .await?;
            sqlx::query("CREATE TRIGGER cards_touch_column_update AFTER UPDATE ON cards WHEN NEW.bulk_touch_epoch=OLD.bulk_touch_epoch AND NOT EXISTS(SELECT 1 FROM card_bulk_touches WHERE board_id=NEW.board_id) BEGIN UPDATE columns SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.column_id; UPDATE columns SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=OLD.column_id AND OLD.column_id IS NOT NEW.column_id; END")
                .execute(&mut *tx).await?;
            tx.commit().await?;
        }
        let old_delete_trigger: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM sqlite_master WHERE type='trigger' AND name='cards_touch_board_delete'",
        )
        .fetch_optional(&db)
        .await?;
        if old_delete_trigger.is_some() {
            sqlx::query("DROP TRIGGER cards_touch_board_delete")
                .execute(&db)
                .await?;
        }
        let closure_columns = sqlx::query("PRAGMA table_info(closures)")
            .fetch_all(&db)
            .await?;
        if !closure_columns
            .iter()
            .any(|row| row.get::<String, _>("name") == "id")
        {
            let mut tx = db.begin().await?;
            sqlx::query("CREATE TABLE closures_rebuilt (id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE, card_id TEXT NOT NULL UNIQUE REFERENCES cards(id) ON DELETE CASCADE, user_id TEXT REFERENCES users(id) ON DELETE SET NULL, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)")
            .execute(&mut *tx).await?;
            let legacy = sqlx::query("SELECT z.card_id,c.account_id,z.user_id,z.created_at FROM closures z JOIN cards c ON c.id=z.card_id")
            .fetch_all(&mut *tx).await?;
            for row in legacy {
                let timestamp: String = row.get("created_at");
                sqlx::query("INSERT INTO closures_rebuilt(id,account_id,card_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
                .bind(id())
                .bind(row.get::<String, _>("account_id"))
                .bind(row.get::<String, _>("card_id"))
                .bind(row.get::<Option<String>, _>("user_id"))
                .bind(&timestamp)
                .bind(&timestamp)
                .execute(&mut *tx).await?;
            }
            sqlx::query("DROP TABLE closures").execute(&mut *tx).await?;
            sqlx::query("ALTER TABLE closures_rebuilt RENAME TO closures")
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        sqlx::query(
            "CREATE INDEX IF NOT EXISTS closures_user_created ON closures(user_id,created_at)",
        )
        .execute(&db)
        .await?;
        sqlx::query(
        "CREATE INDEX IF NOT EXISTS closures_account_created ON closures(account_id,created_at)",
    )
    .execute(&db)
    .await?;
        let spike_columns = sqlx::query("PRAGMA table_info(card_activity_spikes)")
            .fetch_all(&db)
            .await?;
        if !spike_columns
            .iter()
            .any(|row| row.get::<String, _>("name") == "id")
        {
            sqlx::query("ALTER TABLE card_activity_spikes ADD COLUMN id TEXT")
                .execute(&db)
                .await?;
        }
        let spikes_missing_ids =
            sqlx::query("SELECT card_id FROM card_activity_spikes WHERE id IS NULL")
                .fetch_all(&db)
                .await?;
        if !spikes_missing_ids.is_empty() {
            let mut tx = db.begin().await?;
            for row in spikes_missing_ids {
                sqlx::query("UPDATE card_activity_spikes SET id=? WHERE card_id=? AND id IS NULL")
                    .bind(uuid::Uuid::now_v7().to_string())
                    .bind(row.get::<String, _>("card_id"))
                    .execute(&mut *tx)
                    .await?;
            }
            tx.commit().await?;
        }
        let spike_fks = sqlx::query("PRAGMA foreign_key_list(card_activity_spikes)")
            .fetch_all(&db)
            .await?;
        if spike_fks
            .iter()
            .any(|row| row.get::<String, _>("table") == "cards")
        {
            let mut tx = db.begin().await?;
            sqlx::query("DROP TRIGGER IF EXISTS cards_delete_activity_spikes")
                .execute(&mut *tx)
                .await?;
            sqlx::query("CREATE TABLE card_activity_spikes_rebuilt (id TEXT PRIMARY KEY, account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE, card_id TEXT NOT NULL UNIQUE, created_at TEXT NOT NULL, updated_at TEXT NOT NULL)")
            .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO card_activity_spikes_rebuilt(id,account_id,card_id,created_at,updated_at) SELECT id,account_id,card_id,created_at,updated_at FROM card_activity_spikes")
            .execute(&mut *tx).await?;
            sqlx::query("DROP TABLE card_activity_spikes")
                .execute(&mut *tx)
                .await?;
            sqlx::query("ALTER TABLE card_activity_spikes_rebuilt RENAME TO card_activity_spikes")
                .execute(&mut *tx)
                .await?;
            tx.commit().await?;
        }
        sqlx::query("CREATE TRIGGER IF NOT EXISTS cards_delete_activity_spikes AFTER DELETE ON cards BEGIN DELETE FROM card_activity_spikes WHERE card_id=OLD.id; END")
        .execute(&db).await?;
        sqlx::query("CREATE INDEX IF NOT EXISTS card_activity_spikes_account_updated ON card_activity_spikes(account_id,updated_at)")
        .execute(&db).await?;
        let watch_columns = sqlx::query("PRAGMA table_info(watches)")
            .fetch_all(&db)
            .await?;
        for column in ["id", "created_at", "updated_at"] {
            if !watch_columns
                .iter()
                .any(|row| row.get::<String, _>("name") == column)
            {
                sqlx::query(&format!("ALTER TABLE watches ADD COLUMN {column} TEXT"))
                    .execute(&db)
                    .await?;
            }
        }
        let legacy_watches = sqlx::query("SELECT w.card_id,w.user_id,c.created_at FROM watches w JOIN cards c ON c.id=w.card_id WHERE w.id IS NULL OR w.created_at IS NULL OR w.updated_at IS NULL")
        .fetch_all(&db).await?;
        if !legacy_watches.is_empty() {
            let mut tx = db.begin().await?;
            for row in legacy_watches {
                let created_at: String = row.get("created_at");
                sqlx::query("UPDATE watches SET id=COALESCE(id,?),created_at=COALESCE(created_at,?),updated_at=COALESCE(updated_at,created_at,?) WHERE card_id=? AND user_id=?")
                .bind(id()).bind(&created_at).bind(&created_at)
                .bind(row.get::<String, _>("card_id"))
                .bind(row.get::<String, _>("user_id"))
                .execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
        sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS watches_id ON watches(id)")
            .execute(&db)
            .await?;
        sqlx::query("INSERT OR IGNORE INTO user_attachables(user_id,account_id,sgid) SELECT id,account_id,lower(hex(randomblob(32))) FROM users")
        .execute(&db).await?;
        migrate_notification_sources(&db).await?;
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
        let legacy_closures = sqlx::query("SELECT c.id,c.account_id,COALESCE((SELECT e.creator_id FROM events e WHERE e.card_id=c.id AND e.action='card_closed' ORDER BY e.created_at DESC,e.id DESC LIMIT 1),c.creator_id) AS user_id,COALESCE((SELECT e.created_at FROM events e WHERE e.card_id=c.id AND e.action='card_closed' ORDER BY e.created_at DESC,e.id DESC LIMIT 1),c.updated_at) AS created_at FROM cards c WHERE c.closed=1 AND NOT EXISTS(SELECT 1 FROM closures z WHERE z.card_id=c.id)")
        .fetch_all(&db).await?;
        if !legacy_closures.is_empty() {
            let mut tx = db.begin().await?;
            for row in legacy_closures {
                let timestamp: String = row.get("created_at");
                sqlx::query("INSERT OR IGNORE INTO closures(id,account_id,card_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
                .bind(id())
                .bind(row.get::<String, _>("account_id"))
                .bind(row.get::<String, _>("id"))
                .bind(row.get::<String, _>("user_id"))
                .bind(&timestamp)
                .bind(&timestamp)
                .execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
        let legacy_goldnesses = sqlx::query("SELECT c.id,c.account_id,c.updated_at FROM cards c WHERE c.golden=1 AND NOT EXISTS(SELECT 1 FROM card_goldnesses g WHERE g.card_id=c.id)")
        .fetch_all(&db).await?;
        if !legacy_goldnesses.is_empty() {
            let mut tx = db.begin().await?;
            for row in legacy_goldnesses {
                let timestamp: String = row.get("updated_at");
                sqlx::query("INSERT OR IGNORE INTO card_goldnesses(id,account_id,card_id,created_at,updated_at) VALUES(?,?,?,?,?)")
                .bind(uuid::Uuid::now_v7().to_string())
                .bind(row.get::<String,_>("account_id"))
                .bind(row.get::<String,_>("id"))
                .bind(&timestamp)
                .bind(&timestamp)
                .execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
        let legacy_not_nows = sqlx::query("SELECT c.id,c.account_id,COALESCE((SELECT e.creator_id FROM events e WHERE e.card_id=c.id AND e.action IN ('card_postponed','card_auto_postponed') ORDER BY e.created_at DESC,e.id DESC LIMIT 1),c.creator_id) AS user_id,COALESCE((SELECT e.created_at FROM events e WHERE e.card_id=c.id AND e.action IN ('card_postponed','card_auto_postponed') ORDER BY e.created_at DESC,e.id DESC LIMIT 1),c.updated_at) AS created_at FROM cards c WHERE c.postponed=1 AND NOT EXISTS(SELECT 1 FROM card_not_nows n WHERE n.card_id=c.id)")
        .fetch_all(&db).await?;
        if !legacy_not_nows.is_empty() {
            let mut tx = db.begin().await?;
            for row in legacy_not_nows {
                let timestamp: String = row.get("created_at");
                sqlx::query("INSERT OR IGNORE INTO card_not_nows(id,account_id,card_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
                .bind(uuid::Uuid::now_v7().to_string())
                .bind(row.get::<String,_>("account_id"))
                .bind(row.get::<String,_>("id"))
                .bind(row.get::<String,_>("user_id"))
                .bind(&timestamp)
                .bind(&timestamp)
                .execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
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
        if !user_columns
            .iter()
            .any(|r| r.get::<String, _>("name") == "updated_at")
        {
            sqlx::query("ALTER TABLE users ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''")
                .execute(&db)
                .await?;
        }
        sqlx::query("UPDATE users SET updated_at=created_at WHERE updated_at=''")
            .execute(&db)
            .await?;
        migrate_deactivated_users(&db).await?;
        backfill_system_users(&db).await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS users_updated_at_insert AFTER INSERT ON users WHEN NEW.updated_at='' BEGIN UPDATE users SET updated_at=NEW.created_at WHERE id=NEW.id; END")
        .execute(&db)
        .await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS users_updated_at_change AFTER UPDATE OF name,role,active,verified_at,identity_id ON users WHEN NEW.name IS NOT OLD.name OR NEW.role IS NOT OLD.role OR NEW.active IS NOT OLD.active OR NEW.verified_at IS NOT OLD.verified_at OR NEW.identity_id IS NOT OLD.identity_id BEGIN UPDATE users SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.id; END")
        .execute(&db)
        .await?;
        sqlx::query("INSERT OR IGNORE INTO notification_settings(user_id,account_id) SELECT id,account_id FROM users WHERE role!='system'")
        .execute(&db).await?;
        let legacy_user_settings = sqlx::query("SELECT u.id,u.account_id,u.created_at FROM users u WHERE u.role!='system' AND NOT EXISTS(SELECT 1 FROM user_settings_records r WHERE r.user_id=u.id)")
        .fetch_all(&db).await?;
        if !legacy_user_settings.is_empty() {
            let mut tx = db.begin().await?;
            for row in legacy_user_settings {
                let timestamp: String = row.get("created_at");
                sqlx::query("INSERT INTO user_settings_records(id,account_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?)")
                .bind(id())
                .bind(row.get::<String, _>("account_id"))
                .bind(row.get::<String, _>("id"))
                .bind(&timestamp)
                .bind(&timestamp)
                .execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
        sqlx::query("CREATE TRIGGER IF NOT EXISTS user_settings_user_insert AFTER INSERT ON users WHEN NEW.role!='system' BEGIN INSERT INTO notification_settings(user_id,account_id) VALUES(NEW.id,NEW.account_id); INSERT INTO user_settings_records(id,account_id,user_id,created_at,updated_at) VALUES('0'||substr(lower(hex(randomblob(16))),1,24),NEW.account_id,NEW.id,NEW.created_at,NEW.created_at); END")
        .execute(&db).await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS user_settings_frequency_update AFTER UPDATE OF bundle_email_frequency ON notification_settings WHEN NEW.bundle_email_frequency<>OLD.bundle_email_frequency BEGIN UPDATE user_settings_records SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE user_id=NEW.user_id; END")
        .execute(&db).await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS user_settings_timezone_insert AFTER INSERT ON user_timezones WHEN NEW.timezone_name IS NOT NULL BEGIN UPDATE user_settings_records SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE user_id=NEW.user_id; END")
        .execute(&db).await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS user_settings_timezone_update AFTER UPDATE OF timezone_name ON user_timezones WHEN OLD.timezone_name IS NOT NEW.timezone_name BEGIN UPDATE user_settings_records SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE user_id=NEW.user_id; END")
        .execute(&db).await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS user_settings_timezone_delete AFTER DELETE ON user_timezones BEGIN UPDATE user_settings_records SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE user_id=OLD.user_id; END")
        .execute(&db).await?;
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
        backfill_webhook_trackers(&db, false).await?;
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
        let event_columns = sqlx::query("PRAGMA table_info(events)")
            .fetch_all(&db)
            .await?;
        if !event_columns
            .iter()
            .any(|r| r.get::<String, _>("name") == "updated_at")
        {
            sqlx::query("ALTER TABLE events ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''")
                .execute(&db)
                .await?;
            sqlx::query("UPDATE events SET updated_at=created_at WHERE updated_at=''")
                .execute(&db)
                .await?;
        }
        // Legacy writers and test fixtures may still omit the column on insert.
        sqlx::query("CREATE TRIGGER IF NOT EXISTS events_updated_at_insert AFTER INSERT ON events WHEN NEW.updated_at='' BEGIN UPDATE events SET updated_at=NEW.created_at WHERE id=NEW.id; END")
        .execute(&db)
        .await?;
        let step_columns = sqlx::query("PRAGMA table_info(steps)")
            .fetch_all(&db)
            .await?;
        if !step_columns
            .iter()
            .any(|r| r.get::<String, _>("name") == "updated_at")
        {
            sqlx::query("ALTER TABLE steps ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''")
                .execute(&db)
                .await?;
            sqlx::query("UPDATE steps SET updated_at=created_at WHERE updated_at=''")
                .execute(&db)
                .await?;
        }
        sqlx::query("CREATE TRIGGER IF NOT EXISTS steps_updated_at_insert AFTER INSERT ON steps WHEN NEW.updated_at='' BEGIN UPDATE steps SET updated_at=NEW.created_at WHERE id=NEW.id; END")
        .execute(&db)
        .await?;
        let assignment_columns = sqlx::query("PRAGMA table_info(assignments)")
            .fetch_all(&db)
            .await?;
        let assignment_updated_at_exists = assignment_columns
            .iter()
            .any(|r| r.get::<String, _>("name") == "updated_at");
        if !assignment_updated_at_exists {
            sqlx::query("ALTER TABLE assignments ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''")
                .execute(&db)
                .await?;
        }
        let assignment_update_backfilled: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM app_secrets WHERE name='assignments_updated_at_v1')",
        )
        .fetch_one(&db)
        .await?;
        if !assignment_updated_at_exists || assignment_update_backfilled == 0 {
            sqlx::query("UPDATE assignments SET updated_at=created_at WHERE updated_at=''")
                .execute(&db)
                .await?;
            sqlx::query("INSERT INTO app_secrets(name,value) VALUES('assignments_updated_at_v1','done') ON CONFLICT(name) DO UPDATE SET value='done'")
                .execute(&db)
                .await?;
        }
        sqlx::query("CREATE TRIGGER IF NOT EXISTS assignments_updated_at_insert AFTER INSERT ON assignments WHEN NEW.updated_at='' BEGIN UPDATE assignments SET updated_at=NEW.created_at WHERE id=NEW.id; END")
        .execute(&db)
        .await?;
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
        if !tag_columns
            .iter()
            .any(|r| r.get::<String, _>("name") == "updated_at")
        {
            sqlx::query("ALTER TABLE tags ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''")
                .execute(&db)
                .await?;
        }
        let tag_records_backfilled: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM app_secrets WHERE name='tag_records_v1')",
        )
        .fetch_one(&db)
        .await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS tags_updated_at_insert AFTER INSERT ON tags WHEN NEW.updated_at='' BEGIN UPDATE tags SET updated_at=NEW.created_at WHERE id=NEW.id; END")
            .execute(&db).await?;
        let tagging_columns = sqlx::query("PRAGMA table_info(taggings)")
            .fetch_all(&db)
            .await?;
        for (name, definition) in [
            ("id", "TEXT"),
            ("created_at", "TEXT NOT NULL DEFAULT ''"),
            ("updated_at", "TEXT NOT NULL DEFAULT ''"),
        ] {
            if !tagging_columns
                .iter()
                .any(|r| r.get::<String, _>("name") == name)
            {
                sqlx::query(&format!(
                    "ALTER TABLE taggings ADD COLUMN {name} {definition}"
                ))
                .execute(&db)
                .await?;
            }
        }
        if tag_records_backfilled == 0 {
            sqlx::query("UPDATE tags SET updated_at=created_at WHERE updated_at=''")
                .execute(&db)
                .await?;
            sqlx::query(
                "UPDATE taggings SET id='0'||substr(lower(hex(randomblob(16))),1,24) WHERE id IS NULL",
            ).execute(&db).await?;
            sqlx::query("UPDATE taggings SET created_at=COALESCE((SELECT created_at FROM cards WHERE id=taggings.card_id),?) WHERE created_at=''")
                .bind(now()).execute(&db).await?;
            sqlx::query("UPDATE taggings SET updated_at=created_at WHERE updated_at=''")
                .execute(&db)
                .await?;
        }
        sqlx::query("CREATE UNIQUE INDEX IF NOT EXISTS taggings_id ON taggings(id)")
            .execute(&db)
            .await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS taggings_record_insert AFTER INSERT ON taggings WHEN NEW.id IS NULL OR NEW.created_at='' OR NEW.updated_at='' BEGIN UPDATE taggings SET id=COALESCE(NEW.id,'0'||substr(lower(hex(randomblob(16))),1,24)),created_at=CASE WHEN NEW.created_at='' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE NEW.created_at END,updated_at=CASE WHEN NEW.updated_at='' THEN CASE WHEN NEW.created_at='' THEN strftime('%Y-%m-%dT%H:%M:%fZ','now') ELSE NEW.created_at END ELSE NEW.updated_at END WHERE card_id=NEW.card_id AND tag_id=NEW.tag_id; END")
            .execute(&db).await?;
        if tag_records_backfilled == 0 {
            sqlx::query("INSERT INTO app_secrets(name,value) VALUES('tag_records_v1','done') ON CONFLICT(name) DO UPDATE SET value='done'")
                .execute(&db).await?;
        }
        let interaction_records_backfilled: i64 = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM app_secrets WHERE name='pin_reaction_update_v1')",
        )
        .fetch_one(&db)
        .await?;
        for table in ["pins", "reactions"] {
            let columns = sqlx::query(&format!("PRAGMA table_info({table})"))
                .fetch_all(&db)
                .await?;
            if !columns
                .iter()
                .any(|r| r.get::<String, _>("name") == "updated_at")
            {
                sqlx::query(&format!(
                    "ALTER TABLE {table} ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''"
                ))
                .execute(&db)
                .await?;
            }
            if interaction_records_backfilled == 0 {
                sqlx::query(&format!(
                    "UPDATE {table} SET updated_at=created_at WHERE updated_at=''"
                ))
                .execute(&db)
                .await?;
            }
            sqlx::query(&format!("CREATE TRIGGER IF NOT EXISTS {table}_updated_at_insert AFTER INSERT ON {table} WHEN NEW.updated_at='' BEGIN UPDATE {table} SET updated_at=NEW.created_at WHERE id=NEW.id; END"))
                .execute(&db).await?;
        }
        if interaction_records_backfilled == 0 {
            sqlx::query("INSERT INTO app_secrets(name,value) VALUES('pin_reaction_update_v1','done') ON CONFLICT(name) DO UPDATE SET value='done'")
                .execute(&db).await?;
        }
        let board_columns = sqlx::query("PRAGMA table_info(boards)")
            .fetch_all(&db)
            .await?;
        if !board_columns
            .iter()
            .any(|r| r.get::<String, _>("name") == "public_description")
        {
            sqlx::query(
                "ALTER TABLE boards ADD COLUMN public_description TEXT NOT NULL DEFAULT ''",
            )
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
        let legacy_publications = sqlx::query("SELECT b.id,b.account_id,b.publication_key,b.updated_at FROM boards b WHERE b.publication_key IS NOT NULL AND NOT EXISTS(SELECT 1 FROM board_publications p WHERE p.board_id=b.id)")
        .fetch_all(&db).await?;
        if !legacy_publications.is_empty() {
            let mut tx = db.begin().await?;
            for row in legacy_publications {
                let timestamp: String = row.get("updated_at");
                sqlx::query("INSERT INTO board_publications(id,account_id,board_id,`key`,created_at,updated_at) VALUES(?,?,?,?,?,?)")
                .bind(id())
                .bind(row.get::<String, _>("account_id"))
                .bind(row.get::<String, _>("id"))
                .bind(row.get::<String, _>("publication_key"))
                .bind(&timestamp)
                .bind(&timestamp)
                .execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
        if !board_columns
            .iter()
            .any(|r| r.get::<String, _>("name") == "entropy_override")
        {
            sqlx::query(
                "ALTER TABLE boards ADD COLUMN entropy_override INTEGER NOT NULL DEFAULT 0",
            )
            .execute(&db)
            .await?;
            sqlx::query(
                "UPDATE boards SET entropy_override=1 WHERE auto_postpone_period_in_days!=30",
            )
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
        sqlx::query("DELETE FROM access_records WHERE NOT EXISTS(SELECT 1 FROM accesses x WHERE x.board_id=access_records.board_id AND x.user_id=access_records.user_id)")
        .execute(&db).await?;
        sqlx::query("UPDATE access_records SET account_id=(SELECT b.account_id FROM boards b WHERE b.id=access_records.board_id) WHERE EXISTS(SELECT 1 FROM boards b WHERE b.id=access_records.board_id AND b.account_id<>access_records.account_id)")
        .execute(&db).await?;
        let legacy_accesses = sqlx::query("SELECT x.board_id,x.user_id,b.account_id,b.created_at,COALESCE(x.accessed_at,b.created_at) AS updated_at FROM accesses x JOIN boards b ON b.id=x.board_id WHERE NOT EXISTS(SELECT 1 FROM access_records r WHERE r.board_id=x.board_id AND r.user_id=x.user_id)")
        .fetch_all(&db).await?;
        if !legacy_accesses.is_empty() {
            let mut tx = db.begin().await?;
            for row in legacy_accesses {
                sqlx::query("INSERT INTO access_records(board_id,user_id,id,account_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
                .bind(row.get::<String, _>("board_id"))
                .bind(row.get::<String, _>("user_id"))
                .bind(id())
                .bind(row.get::<String, _>("account_id"))
                .bind(row.get::<String, _>("created_at"))
                .bind(row.get::<String, _>("updated_at"))
                .execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
        sqlx::query("CREATE TRIGGER IF NOT EXISTS access_records_insert AFTER INSERT ON accesses BEGIN INSERT INTO access_records(board_id,user_id,id,account_id,created_at,updated_at) VALUES(NEW.board_id,NEW.user_id,'0'||substr(lower(hex(randomblob(16))),1,24),(SELECT account_id FROM boards WHERE id=NEW.board_id),strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')); END")
        .execute(&db).await?;
        let access_update_sql = "CREATE TRIGGER access_records_update AFTER UPDATE OF involvement,accessed_at ON accesses WHEN NEW.involvement IS NOT OLD.involvement OR NEW.accessed_at IS NOT OLD.accessed_at BEGIN UPDATE access_records SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE board_id=NEW.board_id AND user_id=NEW.user_id; END";
        let access_update_existing: Option<String> = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='trigger' AND name='access_records_update'",
        )
        .fetch_optional(&db)
        .await?;
        if access_update_existing
            .as_deref()
            .is_none_or(|sql| !sql.contains("WHEN NEW.involvement IS NOT OLD.involvement"))
        {
            let mut tx = db.begin().await?;
            sqlx::query("DROP TRIGGER IF EXISTS access_records_update")
                .execute(&mut *tx)
                .await?;
            sqlx::query(access_update_sql).execute(&mut *tx).await?;
            tx.commit().await?;
        }
        sqlx::query("INSERT OR IGNORE INTO accesses(board_id,user_id,involvement) SELECT b.id,u.id,CASE WHEN b.creator_id=u.id THEN 'watching' ELSE 'access_only' END FROM boards b JOIN users u ON u.account_id=b.account_id WHERE b.all_access=1 AND u.active=1 AND u.role!='system'")
        .execute(&db).await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS accesses_touch_related_insert AFTER INSERT ON accesses BEGIN UPDATE boards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.board_id; UPDATE users SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.user_id; END")
        .execute(&db).await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS accesses_touch_related_update AFTER UPDATE OF involvement,accessed_at ON accesses WHEN NEW.involvement IS NOT OLD.involvement OR NEW.accessed_at IS NOT OLD.accessed_at BEGIN UPDATE boards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.board_id; UPDATE users SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.user_id; END")
        .execute(&db).await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS accesses_touch_related_delete AFTER DELETE ON accesses BEGIN UPDATE boards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=OLD.board_id; UPDATE users SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=OLD.user_id; END")
        .execute(&db).await?;
        let account_columns = sqlx::query("PRAGMA table_info(accounts)")
            .fetch_all(&db)
            .await?;
        if !account_columns
            .iter()
            .any(|r| r.get::<String, _>("name") == "updated_at")
        {
            sqlx::query("ALTER TABLE accounts ADD COLUMN updated_at TEXT NOT NULL DEFAULT ''")
                .execute(&db)
                .await?;
        }
        sqlx::query("UPDATE accounts SET updated_at=created_at WHERE updated_at=''")
            .execute(&db)
            .await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS accounts_updated_at_insert AFTER INSERT ON accounts WHEN NEW.updated_at='' BEGIN UPDATE accounts SET updated_at=NEW.created_at WHERE id=NEW.id; END")
        .execute(&db)
        .await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS accounts_updated_at_name AFTER UPDATE OF name ON accounts WHEN NEW.name IS NOT OLD.name BEGIN UPDATE accounts SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE id=NEW.id; END")
        .execute(&db)
        .await?;
        if !account_columns
            .iter()
            .any(|r| r.get::<String, _>("name") == "auto_postpone_period_in_days")
        {
            sqlx::query("ALTER TABLE accounts ADD COLUMN auto_postpone_period_in_days INTEGER NOT NULL DEFAULT 30")
            .execute(&db).await?;
        }
        let legacy_entropies = sqlx::query("SELECT a.id AS container_id,a.id AS account_id,'Account' AS container_type,a.created_at AS timestamp FROM accounts a WHERE NOT EXISTS(SELECT 1 FROM entropy_records e WHERE e.container_type='Account' AND e.container_id=a.id) UNION ALL SELECT b.id,b.account_id,'Board',b.updated_at FROM boards b WHERE b.entropy_override=1 AND NOT EXISTS(SELECT 1 FROM entropy_records e WHERE e.container_type='Board' AND e.container_id=b.id)")
        .fetch_all(&db).await?;
        if !legacy_entropies.is_empty() {
            let mut tx = db.begin().await?;
            for row in legacy_entropies {
                let timestamp: String = row.get("timestamp");
                sqlx::query("INSERT INTO entropy_records(id,account_id,container_type,container_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
                .bind(id())
                .bind(row.get::<String, _>("account_id"))
                .bind(row.get::<String, _>("container_type"))
                .bind(row.get::<String, _>("container_id"))
                .bind(&timestamp)
                .bind(&timestamp)
                .execute(&mut *tx).await?;
            }
            tx.commit().await?;
        }
        sqlx::query("CREATE TRIGGER IF NOT EXISTS entropy_records_account_insert AFTER INSERT ON accounts BEGIN INSERT INTO entropy_records(id,account_id,container_type,container_id,created_at,updated_at) VALUES('0'||substr(lower(hex(randomblob(16))),1,24),NEW.id,'Account',NEW.id,NEW.created_at,NEW.created_at); END")
        .execute(&db).await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS entropy_records_account_update AFTER UPDATE OF auto_postpone_period_in_days ON accounts WHEN NEW.auto_postpone_period_in_days<>OLD.auto_postpone_period_in_days BEGIN UPDATE entropy_records SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE container_type='Account' AND container_id=NEW.id; UPDATE cards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE account_id=NEW.id; END")
        .execute(&db).await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS entropy_records_board_insert AFTER INSERT ON boards WHEN NEW.entropy_override=1 BEGIN INSERT INTO entropy_records(id,account_id,container_type,container_id,created_at,updated_at) VALUES('0'||substr(lower(hex(randomblob(16))),1,24),NEW.account_id,'Board',NEW.id,NEW.created_at,NEW.created_at); END")
        .execute(&db).await?;
        sqlx::query("CREATE TRIGGER IF NOT EXISTS entropy_records_board_update AFTER UPDATE OF auto_postpone_period_in_days,entropy_override ON boards WHEN NEW.entropy_override=1 AND (OLD.entropy_override=0 OR NEW.auto_postpone_period_in_days<>OLD.auto_postpone_period_in_days) BEGIN INSERT INTO entropy_records(id,account_id,container_type,container_id,created_at,updated_at) VALUES('0'||substr(lower(hex(randomblob(16))),1,24),NEW.account_id,'Board',NEW.id,strftime('%Y-%m-%dT%H:%M:%fZ','now'),strftime('%Y-%m-%dT%H:%M:%fZ','now')) ON CONFLICT(container_type,container_id) DO UPDATE SET updated_at=excluded.updated_at; UPDATE cards SET updated_at=strftime('%Y-%m-%dT%H:%M:%fZ','now') WHERE board_id=NEW.id; END")
        .execute(&db).await?;
        sqlx::query(
            "CREATE UNIQUE INDEX IF NOT EXISTS boards_publication_key ON boards(publication_key)",
        )
        .execute(&db)
        .await?;
        rich_text_records::backfill(&db, false).await?;
        let missing_join_codes=sqlx::query("SELECT a.id FROM accounts a WHERE NOT EXISTS(SELECT 1 FROM account_join_codes j WHERE j.account_id=a.id)")
        .fetch_all(&db).await?;
        for account in missing_join_codes {
            let mut tx = db.begin().await?;
            insert_join_code_tx(&mut tx, &account.get::<String, _>("id"), false).await?;
            tx.commit().await?;
        }
        sqlx::query(
            "INSERT INTO app_secrets(name,value) VALUES('qr_codes',?) ON CONFLICT(name) DO NOTHING",
        )
        .bind(random_token())
        .execute(&db)
        .await?;
        let qr_secret: String =
            sqlx::query_scalar("SELECT value FROM app_secrets WHERE name='qr_codes'")
                .fetch_one(&db)
                .await?;
        let board_etag_connection = if url.contains(":memory:") {
            None
        } else {
            Some(tokio::sync::Mutex::new(
                SqliteConnection::connect_with(&options).await?,
            ))
        };
        (db, qr_secret, board_etag_connection)
    };
    detach_legacy_system_identities(&db, mysql).await?;
    if env::args().nth(1).as_deref() == Some("init-db") {
        return Ok(());
    }
    if env::args().nth(1).as_deref() == Some("seed") {
        seed(&db, mysql).await?;
        return Ok(());
    }
    let sgid_secret = env::var("RIZZY_SECRET_KEY_BASE").unwrap_or_else(|_| qr_secret.clone());
    let app = Arc::new(App {
        db,
        mysql,
        qr_secret,
        sgid_signer: signed_global_id::Signer::new(&sgid_secret),
        write_lock: tokio::sync::Mutex::new(()),
        auth_lock: tokio::sync::Mutex::new(()),
        notification_html_cache: tokio::sync::Mutex::new(HashMap::new()),
        notification_html_build_locks: tokio::sync::Mutex::new(HashMap::new()),
        events_html_cache: tokio::sync::Mutex::new(HashMap::new()),
        events_html_build_locks: tokio::sync::Mutex::new(HashMap::new()),
        board_etag_connection,
        board_etag_cache: tokio::sync::Mutex::new(HashMap::new()),
        export_notify: tokio::sync::Notify::new(),
        import_notify: tokio::sync::Notify::new(),
    });
    if env::args().nth(1).as_deref() == Some("cleanup-webhook-deliveries") {
        let count = webhook_cleanup::cleanup(&app)
            .await
            .map_err(|error| std::io::Error::other(error.1))?;
        println!("Cleaned {count} webhook deliveries");
        return Ok(());
    }
    if let Some(command) = env::args().nth(1) {
        if matches!(
            command.as_str(),
            "cleanup-magic-links" | "cleanup-transfers" | "cleanup-exports" | "cleanup-imports"
        ) {
            recurring_cleanup::run_now(&app, &command).await?;
            return Ok(());
        }
    }
    if env::args().nth(1).as_deref() == Some("incinerate-due") {
        account_cancellation::incinerate_due(&app).await?;
        return Ok(());
    }
    if env::args().nth(1).as_deref() == Some("deliver-cancellation-mail") {
        let count = account_cancellation::deliver_due(&app)
            .await
            .map_err(|error| std::io::Error::other(error.1))?;
        println!("Delivered {count} account cancellation emails");
        return Ok(());
    }
    if env::args().nth(1).as_deref() == Some("inspect-webhook") {
        let delivery_id = env::args()
            .nth(2)
            .ok_or("inspect-webhook needs a delivery ID")?;
        let row = sqlx::query("SELECT d.*,w.url,w.signing_secret,e.creator_id,e.created_at event_created_at,a.external_account_id,a.updated_at account_updated_at FROM webhook_deliveries d JOIN webhooks w ON w.id=d.webhook_id JOIN events e ON e.id=d.event_id JOIN accounts a ON a.id=d.account_id WHERE d.id=?")
            .bind(delivery_id).fetch_one(&app.db).await?;
        let host = env::var("RIZZY_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3007".into());
        let (body, format) = webhook_payload(&app, &row, &host)
            .await
            .map_err(|error| std::io::Error::other(error.1))?;
        println!(
            "{}",
            json!({"body":body,"content_type":format.content_type()})
        );
        return Ok(());
    }
    if !mysql {
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
        let card_search_schema: String = sqlx::query_scalar(
            "SELECT sql FROM sqlite_master WHERE type='table' AND name='card_search'",
        )
        .fetch_one(&app.db)
        .await?;
        if !card_search_schema.contains("porter unicode61") {
            let mut tx = begin_write(&app).await?;
            sqlx::raw_sql(
            "DROP TRIGGER IF EXISTS cards_search_insert;
             DROP TRIGGER IF EXISTS cards_search_update;
             DROP TRIGGER IF EXISTS cards_search_delete;
             DROP TRIGGER IF EXISTS comments_search_insert;
             DROP TRIGGER IF EXISTS comments_search_update;
             DROP TRIGGER IF EXISTS comments_search_delete;
             DROP TABLE card_search;
             DROP TABLE comment_search;
             CREATE VIRTUAL TABLE card_search USING fts5(title,description,tokenize='porter unicode61');
             CREATE VIRTUAL TABLE comment_search USING fts5(body,tokenize='porter unicode61');",
        )
        .execute(&mut *tx)
        .await?;
            sqlx::raw_sql(include_str!("../search_triggers.sql"))
                .execute(&mut *tx)
                .await?;
            sqlx::query("INSERT INTO card_search(rowid,title,description) SELECT rowid,title,search_text FROM cards WHERE status='published'")
            .execute(&mut *tx).await?;
            sqlx::query("INSERT INTO comment_search(rowid,body) SELECT m.rowid,m.search_text FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.status='published'")
            .execute(&mut *tx).await?;
            tx.commit().await?;
        }
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
    if env::args().nth(1).as_deref() == Some("materialize-storage") {
        let mut total = 0;
        loop {
            let count = storage::materialize_pending(&app)
                .await
                .map_err(|error| std::io::Error::other(error.1))?;
            total += count;
            if count == 0 {
                break;
            }
        }
        println!("Materialized {total} storage totals");
        return Ok(());
    }
    if env::args().nth(1).as_deref() == Some("reconcile-storage") {
        let count = storage::reconcile_all(&app)
            .await
            .map_err(|error| std::io::Error::other(error.1))?;
        println!("Reconciled {count} storage owners");
        return Ok(());
    }
    if env::args().nth(1).as_deref() == Some("auto-postpone") {
        let started = Instant::now();
        let count = auto_postpone_all_due(&app)
            .await
            .map_err(|error| std::io::Error::other(error.1))?;
        println!("Auto-postponed {count} cards");
        if env::var("RIZZY_BENCH_AUTO_POSTPONE").as_deref() == Ok("1") {
            eprintln!(
                "AUTO_POSTPONE_SECONDS={:.6}",
                started.elapsed().as_secs_f64()
            );
        }
        return Ok(());
    }
    if env::args().nth(1).as_deref() == Some("delete-unused-tags") {
        let count = tag_cleanup::delete_unused_tags(&app)
            .await
            .map_err(|error| std::io::Error::other(error.1))?;
        println!("Deleted {count} unused tags");
        return Ok(());
    }
    if env::args().nth(1).as_deref() == Some("purge-stored-blobs") {
        let limit = if env::args().nth(2).as_deref() == Some("--one") {
            1
        } else {
            100
        };
        let count = storage::purge_pending(&app, limit)
            .await
            .map_err(|error| std::io::Error::other(error.1))?;
        println!("Purged {count} stored blobs");
        return Ok(());
    }
    if env::var("RIZZY_DISABLE_WEBHOOK_DELIVERY").as_deref() != Ok("1") {
        tokio::spawn(webhook_worker(app.clone()));
    }
    if env::var("RIZZY_DISABLE_WEBHOOK_CLEANUP").as_deref() != Ok("1") {
        tokio::spawn(webhook_cleanup::worker(app.clone()));
    }
    tokio::spawn(exports::worker(app.clone()));
    tokio::spawn(imports::worker(app.clone()));
    tokio::spawn(notification_mail::worker(app.clone()));
    tokio::spawn(account_cancellation::worker(app.clone()));
    if env::var("RIZZY_DISABLE_PUSH_DELIVERY").as_deref() != Ok("1") {
        tokio::spawn(push_delivery::worker(app.clone()));
    }
    if env::var("RIZZY_DISABLE_STORAGE_MATERIALIZATION").as_deref() != Ok("1") {
        tokio::spawn(storage::materialize_worker(app.clone()));
    }
    if env::var("RIZZY_DISABLE_STORAGE_PURGE").as_deref() != Ok("1") {
        tokio::spawn(storage::purge_worker(app.clone()));
    }
    let maintenance_db = app.db.clone();
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(3600));
        loop {
            interval.tick().await;
            let expiration = now();
            let attempts_cutoff = (Utc::now() - chrono::Duration::minutes(15))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            for (statement, cutoff) in [
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
        }
    });
    tokio::spawn(recurring_cleanup::worker(app.clone()));
    tokio::spawn(account_cancellation::incineration_worker(app.clone()));
    if env::var("RIZZY_DISABLE_AUTO_POSTPONE").as_deref() != Ok("1") {
        let app = app.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(30));
            let started_utc_minute = Utc::now().timestamp().div_euclid(60);
            loop {
                interval.tick().await;
                let current = Local::now();
                if !recurring_cleanup::after_start_minute(started_utc_minute, &current)
                    || current.minute() != 50
                {
                    continue;
                }
                let slot = current
                    .with_timezone(&Utc)
                    .format("%Y-%m-%dT%H")
                    .to_string();
                match claim_auto_postpone_slot(&app, &slot).await {
                    Ok(true) => {
                        if let Err(error) = auto_postpone_all_due(&app).await {
                            eprintln!("automatic card postponement failed: {}", error.1);
                            if let Err(release) = sqlx::query("UPDATE app_secrets SET value='0' WHERE name='auto_postpone_hour' AND value=?")
                                .bind(&slot).execute(&app.db).await {
                                eprintln!("automatic card postponement claim release failed: {release}");
                            }
                        }
                    }
                    Ok(false) => {}
                    Err(error) => {
                        eprintln!("automatic card postponement claim failed: {}", error.1)
                    }
                }
            }
        });
    }
    if env::var("RIZZY_DISABLE_TAG_CLEANUP").as_deref() != Ok("1") {
        tokio::spawn(tag_cleanup::worker(app.clone()));
    }
    let routes = Router::new()
        .route("/up", get(|| async { "ok" }))
        .route("/rails/action_mailbox/postmark/inbound_emails", post(action_mailbox::unavailable_ingress))
        .route("/rails/action_mailbox/relay/inbound_emails", post(action_mailbox::unavailable_ingress))
        .route("/rails/action_mailbox/sendgrid/inbound_emails", post(action_mailbox::unavailable_ingress))
        .route("/rails/action_mailbox/mandrill/inbound_emails", get(action_mailbox::unavailable_ingress).post(action_mailbox::unavailable_ingress))
        .route("/rails/action_mailbox/mailgun/inbound_emails/mime", post(action_mailbox::unavailable_ingress))
        .route("/rails/conductor/action_mailbox/inbound_emails", get(action_mailbox::gated_conductor).post(action_mailbox::rejected_conductor_write))
        .route("/rails/conductor/action_mailbox/inbound_emails/new", get(action_mailbox::gated_conductor))
        .route("/rails/conductor/action_mailbox/inbound_emails/{id}", get(action_mailbox::gated_conductor))
        .route("/rails/conductor/action_mailbox/inbound_emails/sources/new", get(action_mailbox::gated_conductor))
        .route("/rails/conductor/action_mailbox/inbound_emails/sources", post(action_mailbox::rejected_conductor_write))
        .route("/rails/conductor/action_mailbox/{inbound_email_id}/reroute", post(action_mailbox::rejected_conductor_write))
        .route("/rails/conductor/action_mailbox/{inbound_email_id}/incinerate", post(action_mailbox::rejected_conductor_write))
        .route("/400.html", get(|| async { static_error_page(include_str!("../static/400.html"), StatusCode::OK) }))
        .route("/404.html", get(|| async { static_error_page(include_str!("../static/404.html"), StatusCode::OK) }))
        .route("/406-unsupported-browser.html", get(|| async { static_error_page(include_str!("../static/406-unsupported-browser.html"), StatusCode::OK) }))
        .route("/422.html", get(|| async { static_error_page(include_str!("../static/422.html"), StatusCode::OK) }))
        .route("/500.html", get(|| async { static_error_page(include_str!("../static/500.html"), StatusCode::OK) }))
        .route("/", get(home))
        .route("/{account}/", get(events_page::index))
        .route("/{account}", get(events_page::index))
        .route("/{account}/landing", get(landing))
        .route("/client_configurations/{configuration}", get(client_config))
        .route("/recede_historical_location", get(|| async { Html("Going back…") }))
        .route("/resume_historical_location", get(|| async { Html("Staying put…") }))
        .route("/refresh_historical_location", get(|| async { Html("Refreshing…") }))
        .route("/{account}/recede_historical_location", get(|| async { Html("Going back…") }))
        .route("/{account}/resume_historical_location", get(|| async { Html("Staying put…") }))
        .route("/{account}/refresh_historical_location", get(|| async { Html("Refreshing…") }))
        .route("/collections/{collection_id}/cards/{id}", get(legacy_card_root))
        .route("/collections/{collection_id}/cards/{id}/", get(legacy_card_root))
        .route("/collections/{id}", get(legacy_board_root))
        .route("/collections/{id}/", get(legacy_board_root))
        .route("/public/collections/{id}", get(legacy_public_board_root))
        .route("/public/collections/{id}/", get(legacy_public_board_root))
        .route("/manifest.json", get(pwa::manifest))
        .route(
            "/error.css",
            get(|| async {
                (
                    [
                        (header::CONTENT_TYPE, "text/css; charset=utf-8"),
                        (header::CACHE_CONTROL, "public, max-age=300"),
                        (header::CONTENT_SECURITY_POLICY, STATIC_CSP),
                    ],
                    include_str!("../static/error.css"),
                )
            }),
        )
        .route("/service-worker", get(pwa::service_worker))
        .route("/service-worker.js", get(pwa::service_worker))
        .route("/assets/pwa/{filename}", get(pwa::asset))
        .route("/assets/events/icons/{filename}", get(events_page::icon))
        .route("/assets/card/icons/{filename}", get(card_page::icon))
        .route("/assets/menu/icons/{filename}", get(events_page::menu_icon))
        .route("/app-icon.png", get(pwa::icon))
        .route("/app-icon-192.png", get(pwa::icon))
        .route("/app-icon-192-maskable.png", get(pwa::icon))
        .route("/session", post(request_magic_link).delete(logout_session))
        .route("/session/new", get(new_session))
        .route("/session/passkey", post(passkeys::sign_in))
        .route("/session/menu", get(account_cancellation::menu))
        .route("/my/passkey_challenge", post(passkeys::challenge))
        .route("/my/passkeys", get(passkeys::unscoped_redirect).post(passkeys::unscoped_redirect))
        .route("/{account}/my/passkeys", get(passkeys::index_scoped).post(passkeys::register_scoped))
        .route("/my/passkeys/{id}/edit", get(passkeys::unscoped_redirect))
        .route("/{account}/my/passkeys/{id}/edit", get(passkeys::edit_scoped))
        .route(
            "/my/passkeys/{id}",
            axum::routing::put(passkeys::unscoped_redirect)
                .patch(passkeys::unscoped_redirect)
                .post(passkeys::unscoped_redirect)
                .delete(passkeys::unscoped_redirect),
        )
        .route(
            "/{account}/my/passkeys/{id}",
            axum::routing::put(passkeys::update_scoped)
                .patch(passkeys::update_scoped)
                .post(passkeys::form_update_or_delete_scoped)
                .delete(passkeys::destroy_scoped),
        )
        .route(
            "/session/transfers/{token}",
            get(session_transfer::show)
                .put(session_transfer::redeem)
                .patch(session_transfer::redeem)
                .post(session_transfer::redeem),
        )
        .route("/session/magic_link", get(show_magic_link).post(complete_magic_link))
        .route(
            "/landing",
            get(|| async {
                (StatusCode::FOUND, [(header::LOCATION, "/session/menu")])
                    .into_response()
            }),
        )
        .route("/signup", get(signup_redirect).post(request_signup))
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
            "/{account}/rails/active_storage/blobs/proxy/{signed_id}/{filename}",
            get(storage::proxy_blob),
        )
        .route(
            "/{account}/rails/active_storage/blobs/{signed_id}/{filename}",
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
            "/{account}/rails/active_storage/representations/redirect/{signed_id}/{variation_key}/{filename}",
            get(storage::show_representation),
        )
        .route(
            "/{account}/rails/active_storage/representations/proxy/{signed_id}/{variation_key}/{filename}",
            get(storage::proxy_representation),
        )
        .route(
            "/{account}/rails/active_storage/representations/{signed_id}/{variation_key}/{filename}",
            get(storage::show_representation),
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
            "/{account}/client_configurations/{configuration}",
            get(client_config),
        )
        .route(
            "/{account}/collections/{collection_id}/cards/{id}",
            get(legacy_card),
        )
        .route(
            "/{account}/collections/{collection_id}/cards/{id}/",
            get(legacy_card),
        )
        .route("/{account}/collections/{id}", get(legacy_board))
        .route("/{account}/collections/{id}/", get(legacy_board))
        .route(
            "/{account}/public/collections/{id}",
            get(legacy_public_board),
        )
        .route(
            "/{account}/public/collections/{id}/",
            get(legacy_public_board),
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
            axum::routing::put(update_board_involvement)
                .patch(update_board_involvement)
                .post(update_board_involvement),
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
        .route("/{account}/public/boards/{key}", get(public_pages::board))
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
            get(public_pages::card),
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
                .post(card_form_method)
                .put(update_card)
                .patch(update_card)
                .delete(delete_card)
                .layer(axum::extract::DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
        .route("/{account}/cards/{number}/draft", get(show_draft))
        .route("/{account}/cards/{number}/edit", get(edit_card))
        .route("/{account}/cards/{number}/revision", get(card_live::current))
        .route("/{account}/cards/{number}/live", get(card_live::stream))
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
        .fallback(static_not_found)
        .method_not_allowed_fallback(unsupported_method)
        .layer(middleware::from_fn_with_state(
            app.clone(),
            register_pwa_on_pages,
        ))
        .layer(middleware::from_fn_with_state(
            app.clone(),
            allow_modern_browser,
        ))
        .layer(middleware::from_fn_with_state(
            app.clone(),
            redirect_tenanted_public_route,
        ))
        .with_state(app.clone())
        .layer(middleware::from_fn(block_search_engine_indexing))
        .layer(middleware::from_fn_with_state(
            app.clone(),
            prevent_request_forgery,
        ))
        .layer(middleware::from_fn(mark_request_format))
        .layer(middleware::from_fn(html_error_contract))
        .layer(middleware::from_fn(content_security_policy))
        .layer(middleware::from_fn(buffered_response_etag))
        .layer(middleware::from_fn(assign_request_id));
    let bind = env::var("RIZZY_BIND").unwrap_or_else(|_| "127.0.0.1:3007".into());
    if dev_magic_links() && !bind.starts_with("127.0.0.1:") && !bind.starts_with("[::1]:") {
        return Err("RIZZY_DEV_MAGIC_LINKS requires a loopback bind address".into());
    }
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("Rizzy listening on http://{bind}");
    let routes = MapRequestLayer::new(rewrite_json_uri).layer(routes);
    let routes = MapResponseLayer::new(|mut response: Response| {
        // Axum adds Allow after a method fallback has returned. Fizzy's routed
        // error and authentication responses do not advertise the literal
        // route's methods.
        if response.status() == StatusCode::NOT_FOUND
            || response
                .extensions()
                .get::<ShadowedMethodResponse>()
                .is_some()
        {
            response.headers_mut().remove(header::ALLOW);
        }
        response
    })
    .layer(routes);
    axum::serve(
        listener,
        routes.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;
    Ok(())
}

async fn seed(db: &DbPool, mysql: bool) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let account_id = id();
    let identity_id = id();
    let user_id = id();
    let system_id = id();
    let token_id = id();
    let timestamp = now();
    let email = env::var("RIZZY_SEED_EMAIL").unwrap_or_else(|_| "owner@example.com".into());
    let name = env::var("RIZZY_SEED_NAME").unwrap_or_else(|_| "Owner".into());
    let account = env::var("RIZZY_SEED_ACCOUNT").unwrap_or_else(|_| "Rizzy".into());
    let token = hex::encode(rand::random::<[u8; 32]>());
    let slug = account_sequence::next(db, mysql).await?;
    let mut tx = db.begin().await?;
    sqlx::query("INSERT INTO accounts(id,external_account_id,name,created_at) VALUES(?,?,?,?)")
        .bind(&account_id)
        .bind(slug)
        .bind(account)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;
    insert_join_code_tx(&mut tx, &account_id, mysql).await?;
    sqlx::query("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)")
        .bind(&identity_id)
        .bind(email)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO users(id,account_id,identity_id,name,role,created_at,verified_at) VALUES(?,?,?,?,'owner',?,?)").bind(&user_id).bind(&account_id).bind(&identity_id).bind(name).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await?;
    sqlx::query(
        "INSERT INTO users(id,account_id,name,role,created_at) VALUES(?,?,'System','system',?)",
    )
    .bind(&system_id)
    .bind(&account_id)
    .bind(&timestamp)
    .execute(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES(?,?,?,?,?,?)").bind(&token_id).bind(&identity_id).bind(hash(&token)).bind("Bootstrap token").bind("write").bind(&timestamp).execute(&mut *tx).await?;
    tx.commit().await?;
    println!("Account: {slug}\nBearer token: {token}");
    Ok(())
}

async fn backfill_system_users(db: &DbPool) -> std::result::Result<(), sqlx::Error> {
    let accounts = sqlx::query("SELECT a.id FROM accounts a WHERE NOT EXISTS(SELECT 1 FROM users u WHERE u.account_id=a.id AND u.role='system')")
        .fetch_all(db).await?;
    for account in accounts {
        let account_id: String = account.get("id");
        let timestamp = now();
        sqlx::query("INSERT OR IGNORE INTO users(id,account_id,name,role,created_at) VALUES(?,?,'System','system',?)")
            .bind(id()).bind(&account_id).bind(&timestamp)
            .execute(db).await?;
    }
    Ok(())
}

async fn detach_legacy_system_identities(
    db: &DbPool,
    mysql: bool,
) -> std::result::Result<(), sqlx::Error> {
    let already_done: i64 = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM app_secrets WHERE name='system_identity_detached_v1')",
    )
    .fetch_one(db)
    .await?;
    if already_done != 0 {
        return Ok(());
    }
    let rows = sqlx::query("SELECT u.id,u.account_id,u.identity_id,u.updated_at,i.email_address FROM users u JOIN identities i ON i.id=u.identity_id WHERE u.role='system'")
        .fetch_all(db).await?;
    for row in rows {
        let account_id: String = row.get("account_id");
        let email: String = row.get("email_address");
        if email != format!("system+{account_id}@rizzy.invalid") {
            continue;
        }
        let user_id: String = row.get("id");
        let identity_id: String = row.get("identity_id");
        let updated_at: String = row.get("updated_at");
        let mut tx = db.begin().await?;
        sqlx::query("UPDATE users SET identity_id=NULL WHERE id=?")
            .bind(&user_id)
            .execute(&mut *tx)
            .await?;
        sqlx::query("UPDATE users SET updated_at=? WHERE id=?")
            .bind(updated_at)
            .bind(&user_id)
            .execute(&mut *tx)
            .await?;
        let references = [
            ("users", "identity_id"),
            ("identity_transfers", "identity_id"),
            ("access_tokens", "identity_id"),
            ("access_token_reveals", "identity_id"),
            ("magic_links", "identity_id"),
            ("email_change_requests", "old_identity_id"),
            ("sessions", "identity_id"),
            ("passkeys", "identity_id"),
            ("passkey_challenges", "identity_id"),
            ("imports", "identity_id"),
        ];
        let predicates = references
            .iter()
            .map(|(table, column)| format!("EXISTS(SELECT 1 FROM {table} WHERE {column}=?)"))
            .collect::<Vec<_>>()
            .join(" OR ");
        let reference_query = format!("SELECT {predicates}");
        let mut check = sqlx::query_scalar::<_, i64>(&reference_query);
        for _ in references {
            check = check.bind(&identity_id);
        }
        if check.fetch_one(&mut *tx).await? == 0 {
            sqlx::query("DELETE FROM identities WHERE id=?")
                .bind(&identity_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
    }
    let insert = if mysql {
        "INSERT IGNORE INTO app_secrets(name,value) VALUES('system_identity_detached_v1','done')"
    } else {
        "INSERT OR IGNORE INTO app_secrets(name,value) VALUES('system_identity_detached_v1','done')"
    };
    sqlx::query(insert).execute(db).await?;
    Ok(())
}

async fn migrate_deactivated_users(db: &DbPool) -> std::result::Result<(), sqlx::Error> {
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
        sqlx::query("CREATE TABLE users_identity_v2 (id TEXT PRIMARY KEY,account_id TEXT NOT NULL REFERENCES accounts(id),identity_id TEXT REFERENCES identities(id),name TEXT NOT NULL,role TEXT NOT NULL DEFAULT 'member',active INTEGER NOT NULL DEFAULT 1,verified_at TEXT,created_at TEXT NOT NULL,updated_at TEXT NOT NULL DEFAULT '',UNIQUE(account_id,identity_id))")
            .execute(&mut *connection).await?;
        sqlx::query("INSERT INTO users_identity_v2(id,account_id,identity_id,name,role,active,verified_at,created_at,updated_at) SELECT id,account_id,identity_id,name,role,active,verified_at,created_at,updated_at FROM users")
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

async fn migrate_notification_sources(db: &DbPool) -> std::result::Result<(), sqlx::Error> {
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
    if app.mysql {
        let mut connection = app.db.acquire().await.map_err(db_err)?;
        let digest = hash(bucket);
        let lock_name = format!("rizzy_auth:{}", &digest[..48]);
        let acquired: Option<i64> = sqlx::query_scalar("SELECT GET_LOCK(?,30)")
            .bind(&lock_name)
            .fetch_one(&mut *connection)
            .await
            .map_err(db_err)?;
        if acquired != Some(1) {
            return Err(err(
                StatusCode::SERVICE_UNAVAILABLE,
                "Authentication rate limiter unavailable",
            ));
        }
        let outcome: Result<()> = async {
            let count: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM authentication_attempts WHERE bucket=? AND created_at>=?",
            )
            .bind(bucket)
            .bind(&cutoff)
            .fetch_one(&mut *connection)
            .await
            .map_err(db_err)?;
            if count >= limit {
                return Err(err(StatusCode::TOO_MANY_REQUESTS, message));
            }
            sqlx::query("INSERT INTO authentication_attempts(id,bucket,created_at) VALUES(?,?,?)")
                .bind(id())
                .bind(bucket)
                .bind(now())
                .execute(&mut *connection)
                .await
                .map_err(db_err)?;
            Ok(())
        }
        .await;
        let released: std::result::Result<Option<i64>, sqlx::Error> =
            sqlx::query_scalar("SELECT RELEASE_LOCK(?)")
                .bind(&lock_name)
                .fetch_one(&mut *connection)
                .await;
        if !matches!(released, Ok(Some(1))) {
            connection.close_on_drop();
            return Err(err(
                StatusCode::SERVICE_UNAVAILABLE,
                "Authentication rate limiter unavailable",
            ));
        }
        outcome
    } else {
        let mut tx = begin_write(app).await.map_err(db_err)?;
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM authentication_attempts WHERE bucket=? AND created_at>=?",
        )
        .bind(bucket)
        .bind(&cutoff)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
        if count >= limit {
            return Err(err(StatusCode::TOO_MANY_REQUESTS, message));
        }
        sqlx::query("INSERT INTO authentication_attempts(id,bucket,created_at) VALUES(?,?,?)")
            .bind(id())
            .bind(bucket)
            .bind(now())
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        tx.commit().await.map_err(db_err)?;
        Ok(())
    }
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
        return Ok(auth_flash_redirect(
            "/session/new",
            "session_missing_email_alert",
            &headers,
        ));
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
        return Ok(auth_flash_redirect(
            "/session/new",
            "session_missing_email_alert",
            &headers,
        ));
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
    let alert = cookie_value(&headers, "magic_link_rate_alert").as_deref() == Some("1");
    let shake = cookie_value(&headers, "magic_link_shake").as_deref() == Some("1");
    let mut response = signup_page::magic_link(&email, &code_hint, alert, shake).into_response();
    for (present, name) in [
        (alert, "magic_link_rate_alert"),
        (shake, "magic_link_shake"),
    ] {
        if present {
            set_cookie(
                response.headers_mut(),
                name,
                "",
                0,
                secure_request(&headers),
            );
        }
    }
    Ok(response)
}

fn auth_flash_redirect(
    path: &'static str,
    cookie_name: &'static str,
    headers: &HeaderMap,
) -> Response {
    let mut response = (StatusCode::FOUND, [(header::LOCATION, path)]).into_response();
    set_cookie(
        response.headers_mut(),
        cookie_name,
        "1",
        60,
        secure_request(headers),
    );
    response
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
    let rate_limited = cookie_value(&headers, "signup_rate_limit_alert").as_deref() == Some("1");
    let mut response = signup_page::new(rate_limited).into_response();
    if rate_limited {
        set_cookie(
            response.headers_mut(),
            "signup_rate_limit_alert",
            "",
            0,
            secure_request(&headers),
        );
    }
    Ok(response)
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
    let secure = secure_request(&headers);
    let signup_attempt = {
        let _guard = app.auth_lock.lock().await;
        auth_attempt(
            &app,
            &format!("signup:{}", peer.ip()),
            10,
            3,
            "Try again later.",
        )
        .await
    };
    match signup_attempt {
        Ok(()) => {}
        Err(ApiError(StatusCode::TOO_MANY_REQUESTS, message, _))
            if message == "Try again later." =>
        {
            let mut response =
                (StatusCode::FOUND, [(header::LOCATION, "/signup/new")]).into_response();
            set_cookie(
                response.headers_mut(),
                "signup_rate_limit_alert",
                "1",
                60,
                secure,
            );
            return Ok(response);
        }
        Err(error) => return Err(error),
    }
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
    if !forced_signup {
        auth_attempt(
            app,
            &format!("request:{}", peer.ip()),
            10,
            3,
            "Try again later.",
        )
        .await?;
    }
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
            let insert_magic_link = if app.mysql {
                "INSERT IGNORE INTO magic_links(id,identity_id,code,purpose,expires_at,created_at) VALUES(?,?,?,?,?,?)"
            } else {
                "INSERT OR IGNORE INTO magic_links(id,identity_id,code,purpose,expires_at,created_at) VALUES(?,?,?,?,?,?)"
            };
            let inserted = sqlx::query(insert_magic_link)
                .bind(id())
                .bind(&identity_id)
                .bind(&candidate)
                .bind(purpose)
                .bind(&expires)
                .bind(now())
                .execute(&app.db)
                .await
                .map_err(db_err)?;
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
    let Some(pending) = cookie_value(&headers, "pending_authentication_token") else {
        if form {
            return Ok(auth_flash_redirect(
                "/session/new",
                "session_missing_email_alert",
                &headers,
            ));
        }
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "Enter your email address to sign in.",
        ));
    };
    let _guard = app.auth_lock.lock().await;
    let record = sqlx::query(
        "SELECT email_address FROM pending_authentications WHERE token_hash=? AND expires_at>?",
    )
    .bind(hash(&pending))
    .bind(now())
    .fetch_optional(&app.db)
    .await
    .map_err(db_err)?;
    let Some(record) = record else {
        if form {
            return Ok(auth_flash_redirect(
                "/session/new",
                "session_missing_email_alert",
                &headers,
            ));
        }
        return Err(err(
            StatusCode::UNAUTHORIZED,
            "Enter your email address to sign in.",
        ));
    };
    let attempt = auth_attempt(
        &app,
        &format!("verify:{}", peer.ip()),
        10,
        15,
        "Try again in 15 minutes.",
    )
    .await;
    if let Err(error) = attempt {
        if form && error.0 == StatusCode::TOO_MANY_REQUESTS {
            return Ok(auth_flash_redirect(
                "/session/magic_link",
                "magic_link_rate_alert",
                &headers,
            ));
        }
        return Err(error);
    }
    let code = normalize_magic_code(&required(&body, "code")?);
    let link=sqlx::query("SELECT m.id,m.identity_id,m.purpose,i.email_address FROM magic_links m JOIN identities i ON i.id=m.identity_id WHERE m.code=? AND m.expires_at>?")
        .bind(&code).bind(now()).fetch_optional(&app.db).await.map_err(db_err)?;
    let Some(link) = link else {
        if form {
            return Ok(auth_flash_redirect(
                "/session/magic_link",
                "magic_link_shake",
                &headers,
            ));
        }
        return Err(err(StatusCode::UNAUTHORIZED, "Try another code."));
    };
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
        response = if form {
            auth_flash_redirect("/session/new", "session_mismatch_alert", &headers)
        } else {
            err(
                StatusCode::UNAUTHORIZED,
                "Something went wrong. Please try again.",
            )
            .into_response()
        };
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
            join_return_to(&headers).unwrap_or_else(|| "/landing".into())
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
    let slug = account_sequence::next(&app.db, app.mysql)
        .await
        .map_err(db_err)?;
    let mut tx = if app.mysql {
        app.db.begin().await
    } else {
        app.db.begin_with("BEGIN IMMEDIATE").await
    }
    .map_err(db_err)?;
    if app.mysql {
        sqlx::query("SELECT id FROM identities WHERE id=? FOR UPDATE")
            .bind(&identity_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    if let Some(token) = cookie_value(&headers, "session_token") {
        sqlx::query("UPDATE sessions SET signup_pending=0 WHERE token_hash=? AND identity_id=?")
            .bind(hash(&token))
            .bind(&identity_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    let account_id = id();
    let user_id = id();
    let system_id = id();
    let timestamp = now();
    let existing_names: Vec<String> = sqlx::query_scalar(
        "SELECT a.name FROM accounts a JOIN users u ON u.account_id=a.id WHERE u.identity_id=?",
    )
    .bind(&identity_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(db_err)?;
    let account_name = signup_name::generate(&name, &existing_names);
    sqlx::query("INSERT INTO accounts(id,external_account_id,name,created_at) VALUES(?,?,?,?)")
        .bind(&account_id)
        .bind(slug)
        .bind(account_name)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    insert_join_code_tx(&mut tx, &account_id, app.mysql)
        .await
        .map_err(db_err)?;
    sqlx::query("INSERT INTO users(id,account_id,identity_id,name,role,created_at,verified_at) VALUES(?,?,?,?,'owner',?,?)")
        .bind(&user_id).bind(&account_id).bind(&identity_id)
        .bind(&name).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
    sqlx::query(
        "INSERT INTO users(id,account_id,name,role,created_at) VALUES(?,?,'System','system',?)",
    )
    .bind(&system_id)
    .bind(&account_id)
    .bind(&timestamp)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
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
            ApiError(
                StatusCode::FOUND,
                format!("{}/session/new", base(headers)),
                false,
            )
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
    let signup_open = accounts == 0 || env::var("RIZZY_MULTI_TENANT").as_deref() == Ok("1");
    let passkey_button = passkeys::sign_in_button().unwrap_or_default();
    let missing_email =
        cookie_value(&headers, "session_missing_email_alert").as_deref() == Some("1");
    let mismatch = cookie_value(&headers, "session_mismatch_alert").as_deref() == Some("1");
    let message = if mismatch {
        Some("Something went wrong. Please try again.")
    } else if missing_email {
        Some("Enter your email address to sign in.")
    } else {
        None
    };
    let csrf_token = passkey_csrf_token(&app, &headers);
    let html = signup_page::session_new(signup_open, &passkey_button, message)
        .0
        .replacen(
            "</head>",
            &format!("<meta name=\"csrf-token\" content=\"{csrf_token}\"></head>"),
            1,
        );
    let mut response = Html(html).into_response();
    set_passkey_csrf_cookie(&app, &headers, response.headers_mut(), &csrf_token);
    for (present, name) in [
        (missing_email, "session_missing_email_alert"),
        (mismatch, "session_mismatch_alert"),
    ] {
        if present {
            set_cookie(
                response.headers_mut(),
                name,
                "",
                0,
                secure_request(&headers),
            );
        }
    }
    Ok(response)
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
            (header::CONTENT_TYPE, "application/json; charset=utf-8"),
            (header::CACHE_CONTROL, "max-age=60, public"),
        ],
        body,
    )
        .into_response()
}
const STATIC_CSP: &str = "default-src 'none'; img-src 'self'; style-src 'self'; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

struct CspConfig {
    disabled: bool,
    report_only: bool,
    static_policy: String,
    dynamic_policy: String,
}

fn csp_config() -> &'static CspConfig {
    static CONFIG: std::sync::OnceLock<CspConfig> = std::sync::OnceLock::new();
    CONFIG.get_or_init(|| {
        let report_uri = env::var("CSP_REPORT_URI")
            .ok()
            .filter(|value| !value.is_empty());
        let mut static_policy = STATIC_CSP.to_owned();
        if let Some(uri) = &report_uri {
            static_policy.push_str("; report-uri ");
            static_policy.push_str(uri);
        }
        let mut directives = Vec::new();
        for (name, defaults, variable) in [
            ("default-src", "'self'", "CSP_DEFAULT_SRC"),
            ("script-src", "'self'", "CSP_SCRIPT_SRC"),
            ("connect-src", "'self'", "CSP_CONNECT_SRC"),
            ("frame-src", "'self'", "CSP_FRAME_SRC"),
            ("style-src", "'self' 'unsafe-inline'", "CSP_STYLE_SRC"),
            ("img-src", "'self' blob: data: https:", "CSP_IMG_SRC"),
            ("font-src", "'self' data: https:", "CSP_FONT_SRC"),
            ("media-src", "'self' blob: data: https:", "CSP_MEDIA_SRC"),
            ("worker-src", "'self' blob:", "CSP_WORKER_SRC"),
            ("object-src", "'none'", ""),
            ("base-uri", "'none'", ""),
            ("form-action", "'self'", "CSP_FORM_ACTION"),
            ("frame-ancestors", "'self'", "CSP_FRAME_ANCESTORS"),
        ] {
            let mut directive = format!("{name} {defaults}");
            if let Ok(sources) = env::var(variable) {
                for source in sources.split_whitespace() {
                    directive.push(' ');
                    directive.push_str(source);
                }
            }
            if name == "script-src" {
                directive.push_str(" 'nonce-{nonce}'");
            }
            directives.push(directive);
        }
        if let Some(uri) = report_uri {
            directives.push(format!("report-uri {uri}"));
        }
        CspConfig {
            disabled: env::var_os("DISABLE_CSP").is_some(),
            report_only: env::var("CSP_REPORT_ONLY").is_ok_and(|value| value == "true"),
            static_policy,
            dynamic_policy: directives.join("; "),
        }
    })
}

fn dynamic_csp(nonce: &str) -> String {
    csp_config().dynamic_policy.replace("{nonce}", nonce)
}

async fn content_security_policy(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let config = csp_config();
    if response.status() == StatusCode::NOT_MODIFIED {
        response
            .headers_mut()
            .remove(header::CONTENT_SECURITY_POLICY);
        response
            .headers_mut()
            .remove(header::CONTENT_SECURITY_POLICY_REPORT_ONLY);
        return response;
    }
    if config.disabled {
        response
            .headers_mut()
            .remove(header::CONTENT_SECURITY_POLICY);
        response
            .headers_mut()
            .remove(header::CONTENT_SECURITY_POLICY_REPORT_ONLY);
        response.headers_mut().remove("x-rizzy-exceptions-app");
        return response;
    }
    if response
        .headers_mut()
        .remove("x-rizzy-exceptions-app")
        .is_some()
    {
        return response;
    }
    if response
        .headers()
        .contains_key(header::CONTENT_SECURITY_POLICY)
    {
        response
            .headers_mut()
            .remove(header::CONTENT_SECURITY_POLICY);
        response.headers_mut().insert(
            if config.report_only {
                header::CONTENT_SECURITY_POLICY_REPORT_ONLY
            } else {
                header::CONTENT_SECURITY_POLICY
            },
            HeaderValue::from_str(&config.static_policy).expect("valid static CSP"),
        );
        return response;
    }
    let nonce = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        rand::rng().random::<[u8; 16]>(),
    );
    let mut response = if response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value.starts_with("text/html") || value.starts_with("text/vnd.turbo-stream.html")
        }) {
        let (mut parts, body) = response.into_parts();
        let Ok(bytes) = axum::body::to_bytes(body, usize::MAX).await else {
            return (StatusCode::INTERNAL_SERVER_ERROR, "Could not render page").into_response();
        };
        if let Ok(mut html) = String::from_utf8(bytes.to_vec()) {
            if let Some(head_end) = html.find("</head>") {
                html.insert_str(
                    head_end,
                    &format!("<meta name=\"csp-nonce\" content=\"{nonce}\">"),
                );
            }
            static SCRIPT_TAG: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            let pattern = SCRIPT_TAG.get_or_init(|| {
                regex::Regex::new(r"(?i)<script\b([^>]*)>").expect("script tag pattern")
            });
            let rendered = pattern.replace_all(&html, |captures: &regex::Captures<'_>| {
                if captures[1].to_ascii_lowercase().contains("nonce=") {
                    captures[0].to_owned()
                } else {
                    format!("<script nonce=\"{nonce}\"{}>", &captures[1])
                }
            });
            parts.headers.remove(header::CONTENT_LENGTH);
            Response::from_parts(parts, Body::from(rendered.into_owned()))
        } else {
            Response::from_parts(parts, Body::from(bytes))
        }
    } else {
        response
    };
    response.headers_mut().insert(
        if config.report_only {
            header::CONTENT_SECURITY_POLICY_REPORT_ONLY
        } else {
            header::CONTENT_SECURITY_POLICY
        },
        HeaderValue::from_str(&dynamic_csp(&nonce)).expect("valid dynamic CSP"),
    );
    response
}

fn static_error_page(body: &'static str, status: StatusCode) -> Response {
    (
        status,
        [
            (header::CONTENT_TYPE, "text/html; charset=utf-8"),
            (header::CONTENT_SECURITY_POLICY, STATIC_CSP),
        ],
        body,
    )
        .into_response()
}

async fn static_not_found(headers: HeaderMap, OriginalUri(uri): OriginalUri) -> Response {
    if headers.contains_key("x-rizzy-json-request") || uri.path().ends_with(".json") {
        let mut response = (
            StatusCode::NOT_FOUND,
            [(header::CONTENT_TYPE, "application/json; charset=UTF-8")],
            r#"{"status":404,"error":"Not Found"}"#,
        )
            .into_response();
        response
            .headers_mut()
            .insert("x-rizzy-exceptions-app", HeaderValue::from_static("1"));
        response
    } else {
        static_error_page(include_str!("../static/404.html"), StatusCode::NOT_FOUND)
    }
}
enum ShadowedDynamicRoute<'a> {
    Account(&'a str),
    Identity,
}
fn shadowed_dynamic_route<'a>(method: &Method, path: &'a str) -> Option<ShadowedDynamicRoute<'a>> {
    // Rails considers the request method while choosing between literal and
    // parameterized paths. Axum gives the literal path precedence for every
    // method, so these methods must reach the parameterized controller's auth
    // check before its missing-record response.
    let path = path.strip_suffix(".json").unwrap_or(path);
    if path == "/my/access_tokens/new" && *method == Method::DELETE {
        return Some(ShadowedDynamicRoute::Identity);
    }
    let (account, tail) = path.strip_prefix('/')?.split_once('/')?;
    let matches = match tail {
        "boards/new" | "cards/previews" => {
            matches!(*method, Method::PATCH | Method::PUT | Method::DELETE)
        }
        "filters/settings_refresh" | "my/access_tokens/new" => *method == Method::DELETE,
        "users/joins" | "users/verifications" => {
            matches!(
                *method,
                Method::GET | Method::HEAD | Method::PATCH | Method::PUT | Method::DELETE
            )
        }
        _ => false,
    };
    matches.then_some(ShadowedDynamicRoute::Account(account))
}
async fn unsupported_method(
    State(app): Shared,
    method: Method,
    headers: HeaderMap,
    original_accept: Option<Extension<OriginalAccept>>,
    uri: OriginalUri,
) -> Response {
    if let Some(route) = shadowed_dynamic_route(&method, uri.path()) {
        let auth: Result<()> = match route {
            ShadowedDynamicRoute::Account(account) => {
                principal(&app, &headers, account).await.map(|_| ())
            }
            ShadowedDynamicRoute::Identity => identity_auth(&app, &headers).await.map(|_| ()),
        };
        let mut response = match auth {
            Ok(_) => static_not_found(headers, uri).await,
            Err(error) if error.0 == StatusCode::FOUND => {
                let mut response = error.into_response();
                response.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("text/html; charset=utf-8"),
                );
                response
            }
            Err(error) => error.into_response(),
        };
        response.extensions_mut().insert(ShadowedMethodResponse);
        return response;
    }
    let accept = match original_accept.as_ref() {
        Some(Extension(original)) => original.0.as_ref(),
        None => headers.get(header::ACCEPT),
    };
    if accept
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/html"))
    {
        static_error_page(include_str!("../static/404.html"), StatusCode::NOT_FOUND)
    } else {
        static_not_found(headers, uri).await
    }
}
fn client_config_error(status: StatusCode, json: bool) -> Response {
    let (content_type, body) = match (status, json) {
        (StatusCode::NOT_FOUND, true) => (
            "application/json; charset=UTF-8",
            r#"{"status":404,"error":"Not Found"}"#,
        ),
        (StatusCode::INTERNAL_SERVER_ERROR, true) => (
            "application/json; charset=UTF-8",
            r#"{"status":500,"error":"Internal Server Error"}"#,
        ),
        (StatusCode::NOT_FOUND, false) => (
            "text/html; charset=UTF-8",
            include_str!("../static/404.html"),
        ),
        _ => (
            "text/html; charset=UTF-8",
            include_str!("../static/500.html"),
        ),
    };
    let mut response = (status, [(header::CONTENT_TYPE, content_type)], body).into_response();
    if !json {
        response.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            HeaderValue::from_static(STATIC_CSP),
        );
    } else {
        response
            .headers_mut()
            .insert("x-rizzy-exceptions-app", HeaderValue::from_static("1"));
    }
    response
}
async fn client_config(headers: HeaderMap, OriginalUri(uri): OriginalUri) -> Response {
    let name = uri.path().rsplit('/').next().unwrap_or("");
    let (basename, extension) = name.rsplit_once('.').unwrap_or((name, ""));
    let (platform, version) = match basename.split_once("_v") {
        Some(parts) => parts,
        None => return client_config_error(StatusCode::NOT_FOUND, name.ends_with(".json")),
    };
    let valid_route = matches!(platform, "android" | "ios")
        && !version.is_empty()
        && version.bytes().all(|byte| byte.is_ascii_digit());
    let query_format = uri.query().and_then(|query| {
        url::form_urlencoded::parse(query.as_bytes())
            .find(|(key, _)| key == "format")
            .map(|(_, value)| value.into_owned())
    });
    let json = if !extension.is_empty() {
        extension == "json"
    } else if let Some(format) = query_format {
        format == "json"
    } else {
        headers
            .get(header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("application/json"))
    };
    if !valid_route {
        return client_config_error(StatusCode::NOT_FOUND, json);
    }
    match (platform, version, json) {
        ("android", "1", true) => android_config().await,
        ("ios", "1", true) => ios_config().await,
        _ => client_config_error(StatusCode::INTERNAL_SERVER_ERROR, json),
    }
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
fn legacy_redirect(headers: &HeaderMap, segments: &[&str]) -> Result<Response> {
    let mut url = url::Url::parse(&base(headers)).map_err(|_| bad("Invalid host"))?;
    url.path_segments_mut()
        .map_err(|_| bad("Invalid host"))?
        .extend(segments);
    Ok((
        StatusCode::MOVED_PERMANENTLY,
        [
            (header::LOCATION, url.to_string()),
            (header::CACHE_CONTROL, "no-cache".to_owned()),
            (header::CONTENT_TYPE, "text/html; charset=UTF-8".to_owned()),
        ],
    )
        .into_response())
}
fn legacy_account(account: &str) -> Result<()> {
    if account.is_empty() || !account.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(missing());
    }
    Ok(())
}
async fn legacy_board_root(Path(id): Path<String>, headers: HeaderMap) -> Result<Response> {
    legacy_redirect(&headers, &["boards", &id])
}
async fn legacy_card_root(
    Path((_, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    legacy_redirect(&headers, &["cards", &id])
}
async fn legacy_public_board_root(Path(id): Path<String>, headers: HeaderMap) -> Result<Response> {
    legacy_redirect(&headers, &["public", "boards", &id])
}
async fn legacy_board(
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    legacy_account(&account)?;
    legacy_redirect(&headers, &[&account, "boards", &id])
}
async fn legacy_card(
    Path((account, _, id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    legacy_account(&account)?;
    legacy_redirect(&headers, &[&account, "cards", &id])
}
async fn legacy_public_board(
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    legacy_account(&account)?;
    legacy_redirect(&headers, &[&account, "public", "boards", &id])
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
    let statement = if app.mysql {
        "INSERT INTO user_timezones(user_id,account_id,timezone_name) VALUES(?,?,?) ON DUPLICATE KEY UPDATE timezone_name=VALUES(timezone_name)"
    } else {
        "INSERT INTO user_timezones(user_id,account_id,timezone_name) VALUES(?,?,?) ON CONFLICT(user_id) DO UPDATE SET timezone_name=excluded.timezone_name WHERE user_timezones.timezone_name IS NOT excluded.timezone_name"
    };
    sqlx::query(statement)
        .bind(&p.user_id)
        .bind(&p.account_id)
        .bind(timezone)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn my_identity(
    State(app): Shared,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Value>> {
    let (identity_id, _) = identity_auth(&app, &headers).await?;
    check_my_scope(&app, &identity_id, &uri).await?;
    let rows=sqlx::query("SELECT a.id account_id,a.external_account_id,a.name,a.created_at,a.updated_at account_updated_at,u.id user_id,u.role FROM accounts a JOIN users u ON u.account_id=a.id WHERE u.identity_id=? AND u.active=1 AND u.role!='system' AND NOT EXISTS(SELECT 1 FROM account_cancellations c WHERE c.account_id=a.id) ORDER BY a.created_at")
        .bind(&identity_id).fetch_all(&app.db).await.map_err(db_err)?;
    let host = base(&headers);
    let mut accounts = Vec::new();
    for r in rows {
        let slug: i64 = r.get("external_account_id");
        let p = Principal {
            account_id: r.get("account_id"),
            account_updated_at: r.get("account_updated_at"),
            user_id: r.get("user_id"),
            role: r.get("role"),
            permission: "read".into(),
            slug,
            request_id: String::new(),
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
            Html(String::new()),
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
            Html(String::new()),
        )
            .into_response());
    }
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
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
async fn active_user(app: &App, p: &Principal, id: &str) -> Result<DbRow> {
    sqlx::query("SELECT * FROM users WHERE id=? AND account_id=? AND active=1 AND role!='system'")
        .bind(id.trim_end_matches(".json"))
        .bind(&p.account_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)
}
fn conditional_html_response(body: String, headers: &HeaderMap) -> Response {
    let mut digest = Sha256::new();
    digest.update(etag_timezone(headers).unwrap_or_default().as_bytes());
    digest.update([0]);
    digest.update(body.as_bytes());
    let etag = format!("W/\"{}\"", hex::encode(digest.finalize()));
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

fn conditional_board_json_response(value: Value, headers: &HeaderMap, salt: &str) -> Response {
    let body = serde_json::to_vec(&value).expect("JSON value is serializable");
    let mut digest = Sha256::new();
    digest.update(etag_timezone(headers).unwrap_or_default().as_bytes());
    digest.update([0]);
    digest.update(salt.as_bytes());
    digest.update([0]);
    digest.update(&body);
    let etag = format!("W/\"{}\"", hex::encode(digest.finalize()));
    let not_modified = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|candidate| candidate.trim() == etag || candidate.trim() == "*")
        });
    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        let mut response = Response::new(Body::from(body));
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        response
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

async fn board_data_version(app: &App) -> Option<i64> {
    let connection = app.board_etag_connection.as_ref()?;
    let mut connection = connection.lock().await;
    sqlx::query_scalar("PRAGMA data_version")
        .fetch_one(&mut *connection)
        .await
        .ok()
}

fn board_etag_key(
    p: &Principal,
    board_id: &str,
    page: i64,
    expanded: bool,
    headers: &HeaderMap,
) -> String {
    format!(
        "{}:{}:{}:{}:{}:{}",
        p.account_id,
        p.user_id,
        board_id,
        page,
        expanded,
        etag_timezone(headers).unwrap_or_default()
    )
}

async fn cached_board_not_modified(app: &App, key: &str, headers: &HeaderMap) -> Option<Response> {
    let candidates = headers.get(header::IF_NONE_MATCH)?.to_str().ok()?;
    let data_version = board_data_version(app).await?;
    let cache = app.board_etag_cache.lock().await;
    let entry = cache.get(key)?;
    if entry.data_version != data_version
        || !candidates
            .split(',')
            .any(|candidate| candidate.trim() == entry.etag)
    {
        return None;
    }
    let mut response = StatusCode::NOT_MODIFIED.into_response();
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&entry.etag).expect("hex ETag"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=0, private, must-revalidate"),
    );
    Some(response)
}

async fn remember_board_etag(app: &App, key: String, before: Option<i64>, etag: String) {
    let Some(before) = before else { return };
    let Some(after) = board_data_version(app).await else {
        return;
    };
    if before == after {
        let mut cache = app.board_etag_cache.lock().await;
        if cache.len() >= 1024 {
            cache.clear();
        }
        cache.insert(
            key,
            BoardEtagCacheEntry {
                data_version: after,
                etag,
            },
        );
    }
}

fn prompt_user_item(app: &App, p: &Principal, row: &DbRow) -> String {
    let id: String = row.get("id");
    let name: String = row.get("name");
    let sgid = app.sgid_signer.user(&id);
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
    let response = conditional_html_response(
        rows.iter()
            .map(|row| prompt_user_item(&app, &p, row))
            .collect(),
        &headers,
    );
    if headers.contains_key("x-rizzy-json-request") && response.status() != StatusCode::NOT_MODIFIED
    {
        return Ok(source_json_internal_error());
    }
    Ok(response)
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
    let response = conditional_html_response(
        rows.iter()
            .map(|row| prompt_user_item(&app, &p, row))
            .collect(),
        &headers,
    );
    if headers.contains_key("x-rizzy-json-request") && response.status() != StatusCode::NOT_MODIFIED
    {
        return Ok(source_json_internal_error());
    }
    Ok(response)
}

#[derive(Deserialize)]
struct PromptCardQuery {
    filter: Option<String>,
}

fn prompt_card_item(slug: i64, row: &DbRow) -> String {
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
        let search = if app.mysql {
            mysql_search::query(filter, &p.account_id)
        } else {
            fts_query(filter)
        };
        if let Some(search) = search {
            let matched = if app.mysql {
                let table = mysql_search::table(&p.account_id);
                let statement = format!(
                    "SELECT c.number,c.title FROM cards c WHERE c.account_id=? AND c.status='published' AND EXISTS(SELECT 1 FROM accesses a WHERE a.board_id=c.board_id AND a.user_id=?) AND EXISTS(SELECT 1 FROM `{table}` s WHERE s.card_id=c.id AND MATCH(s.account_key,s.content,s.title) AGAINST(? IN BOOLEAN MODE)) ORDER BY c.created_at DESC,c.id DESC LIMIT 10"
                );
                sqlx::query(&statement)
                    .bind(&p.account_id)
                    .bind(&p.user_id)
                    .bind(&search)
                    .fetch_all(&app.db)
                    .await
                    .map_err(db_err)?
            } else {
                sqlx::query("SELECT c.number,c.title FROM cards c JOIN (SELECT id AS card_id FROM cards WHERE rowid IN (SELECT rowid FROM card_search WHERE card_search MATCH ?) UNION ALL SELECT card_id FROM comments WHERE rowid IN (SELECT rowid FROM comment_search WHERE comment_search MATCH ?)) hits ON hits.card_id=c.id WHERE c.account_id=? AND c.status='published' AND EXISTS(SELECT 1 FROM accesses a WHERE a.board_id=c.board_id AND a.user_id=?) ORDER BY c.created_at DESC,c.id DESC LIMIT 10")
                    .bind(&search).bind(&search).bind(&p.account_id).bind(&p.user_id)
                    .fetch_all(&app.db).await.map_err(db_err)?
            };
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
    let response = conditional_html_response(html, &headers);
    if headers.contains_key("x-rizzy-json-request") && response.status() != StatusCode::NOT_MODIFIED
    {
        return Ok(source_json_internal_error());
    }
    Ok(response)
}
async fn list_users(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    if !headers.contains_key("x-rizzy-json-request") {
        return Ok((StatusCode::NOT_ACCEPTABLE, Html(String::new())).into_response());
    }
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
    let p = principal(&app, &headers, &account).await?;
    let u = active_user(&app, &p, &id).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let email: String = sqlx::query_scalar("SELECT email_address FROM identities WHERE id=?")
        .bind(u.get::<String, _>("identity_id"))
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let has_avatar: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM storage_attachments WHERE record_type='User' AND name='avatar' AND record_id=? AND account_id=?")
        .bind(u.get::<String, _>("id"))
        .bind(&p.account_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    Ok(user_page::edit(&p, &u, &email, None, has_avatar > 0))
}
fn can_change_user(p: &Principal, u: &DbRow) -> Result<()> {
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
                "user[avatar]" if !value.is_empty() => {
                    user.insert("avatar".into(), json!(value.into_owned()));
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
            let has_avatar: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM storage_attachments WHERE record_type='User' AND name='avatar' AND record_id=? AND account_id=?")
                .bind(u.get::<String, _>("id"))
                .bind(&p.account_id)
                .fetch_one(&app.db)
                .await
                .map_err(db_err)?;
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                user_page::edit(&p, &u, &email, Some("Name can't be blank"), has_avatar > 0),
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
    let avatar_blob = avatar
        .as_ref()
        .and_then(Option::as_deref)
        .map(str::to_owned);
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
    if let Some(blob_id) = avatar_blob {
        storage::process_user_avatar_variant(&app, &blob_id).await?;
    }
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
    let board_ids: Vec<String> =
        sqlx::query_scalar("SELECT board_id FROM accesses WHERE user_id=?")
            .bind(&id)
            .fetch_all(&mut *tx)
            .await
            .map_err(db_err)?;
    sqlx::query("DELETE FROM accesses WHERE user_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    for board_id in board_ids {
        clean_inaccessible_board_data(&mut tx, &board_id, &id).await?;
    }
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

async fn clean_inaccessible_board_data(
    tx: &mut sqlx::Transaction<'_, Db>,
    board_id: &str,
    user_id: &str,
) -> Result<()> {
    let still_has_access: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM accesses WHERE board_id=? AND user_id=?)")
            .bind(board_id)
            .bind(user_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(db_err)?;
    if still_has_access != 0 {
        return Ok(());
    }
    sqlx::query("DELETE FROM mentions WHERE mentionee_id=? AND ((source_type='Card' AND source_id IN (SELECT id FROM cards WHERE board_id=?)) OR (source_type='Comment' AND source_id IN (SELECT m.id FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.board_id=?)))")
        .bind(user_id).bind(board_id).bind(board_id).execute(&mut **tx).await.map_err(db_err)?;
    sqlx::query("DELETE FROM notifications WHERE user_id=? AND card_id IN (SELECT id FROM cards WHERE board_id=?)")
        .bind(user_id).bind(board_id).execute(&mut **tx).await.map_err(db_err)?;
    sqlx::query("DELETE FROM watches WHERE user_id=? AND card_id IN (SELECT id FROM cards WHERE board_id=?)")
        .bind(user_id).bind(board_id).execute(&mut **tx).await.map_err(db_err)?;
    sqlx::query(
        "DELETE FROM pins WHERE user_id=? AND card_id IN (SELECT id FROM cards WHERE board_id=?)",
    )
    .bind(user_id)
    .bind(board_id)
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;
    sqlx::query("DELETE FROM assignments WHERE assignee_id=? AND card_id IN (SELECT id FROM cards WHERE board_id=?)")
        .bind(user_id).bind(board_id).execute(&mut **tx).await.map_err(db_err)?;
    Ok(())
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
    let p = principal(&app, &headers, &account).await?;
    if p.user_id != user_id {
        return Err(missing());
    }
    active_user(&app, &p, &user_id).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
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
    let insert_identity = if app.mysql {
        "INSERT IGNORE INTO identities(id,email_address,created_at) VALUES(?,?,?)"
    } else {
        "INSERT OR IGNORE INTO identities(id,email_address,created_at) VALUES(?,?,?)"
    };
    sqlx::query(insert_identity)
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
    let update_user = if app.mysql {
        "UPDATE IGNORE users SET identity_id=? WHERE id=? AND account_id=? AND identity_id=? AND active=1"
    } else {
        "UPDATE OR IGNORE users SET identity_id=? WHERE id=? AND account_id=? AND identity_id=? AND active=1"
    };
    let changed = sqlx::query(update_user)
        .bind(&new_identity)
        .bind(&user_id)
        .bind(user.get::<String, _>("account_id"))
        .bind(&old_identity)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
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
fn entropy_days_from_json(value: &Value) -> Option<i64> {
    match value {
        Value::Number(number) => number
            .as_i64()
            .or_else(|| number.as_f64().map(|number| number.trunc() as i64)),
        Value::String(value) => {
            // Entropy's Rails setter calls to_i, accepting a leading integer
            // even when a JSON value is a string such as "7days".
            let value = value.trim_start();
            let mut length = 0;
            for (index, character) in value.char_indices() {
                if index == 0 && matches!(character, '+' | '-') {
                    length = character.len_utf8();
                } else if character.is_ascii_digit() {
                    length = index + character.len_utf8();
                } else {
                    break;
                }
            }
            Some(value[..length].parse::<i64>().unwrap_or(0))
        }
        Value::Null => Some(0),
        Value::Bool(_) | Value::Array(_) | Value::Object(_) => None,
    }
}
fn entropy_parameter_error(status: StatusCode) -> Response {
    match status {
        StatusCode::BAD_REQUEST => {
            (status, Json(json!({"status":400,"error":"Bad Request"}))).into_response()
        }
        StatusCode::INTERNAL_SERVER_ERROR => (
            status,
            Json(json!({"status":500,"error":"Internal Server Error"})),
        )
            .into_response(),
        _ => status.into_response(),
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
        "SELECT j.code,j.usage_count,j.usage_limit,a.name AS account_name FROM account_join_codes j JOIN accounts a ON a.id=j.account_id WHERE j.account_id=?",
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
        let qr_id = session_transfer::qr_link(&app, p.slug, &url);
        let qr_url = format!("/{}/qr_codes/{qr_id}", p.slug);
        let account_name: String = row.get("account_name");
        Ok(join_code_page::show(&p, &url, &qr_url, &account_name, count, limit).into_response())
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
        let statement = if app.mysql {
            "UPDATE IGNORE account_join_codes SET code=?,usage_count=0,updated_at=? WHERE account_id=?"
        } else {
            "UPDATE OR IGNORE account_join_codes SET code=?,usage_count=0,updated_at=? WHERE account_id=?"
        };
        let result = sqlx::query(statement)
            .bind(join_code())
            .bind(now())
            .bind(&p.account_id)
            .execute(&app.db)
            .await
            .map_err(db_err)?;
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
        return Ok((StatusCode::NOT_FOUND, Html(String::new())).into_response());
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
    let insert_identity = if app.mysql {
        "INSERT IGNORE INTO identities(id,email_address,created_at) VALUES(?,?,?)"
    } else {
        "INSERT OR IGNORE INTO identities(id,email_address,created_at) VALUES(?,?,?)"
    };
    sqlx::query(insert_identity)
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
    let insert_user = if app.mysql {
        "INSERT IGNORE INTO users(id,account_id,identity_id,name,role,active,created_at) VALUES(?,?,?,?,'member',1,?)"
    } else {
        "INSERT OR IGNORE INTO users(id,account_id,identity_id,name,role,active,created_at) VALUES(?,?,?,?,'member',1,?)"
    };
    let inserted = sqlx::query(insert_user)
        .bind(id())
        .bind(&account_id)
        .bind(&identity_id)
        .bind(&email)
        .bind(now())
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    if inserted.rows_affected() == 1 {
        if app.mysql {
            let user_id: String =
                sqlx::query_scalar("SELECT id FROM users WHERE account_id=? AND identity_id=?")
                    .bind(&account_id)
                    .bind(&identity_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db_err)?;
            let board_ids: Vec<String> =
                sqlx::query_scalar("SELECT id FROM boards WHERE account_id=? AND all_access=1")
                    .bind(&account_id)
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(db_err)?;
            for board_id in board_ids {
                sqlx::query("INSERT IGNORE INTO accesses(board_id,user_id) VALUES(?,?)")
                    .bind(board_id)
                    .bind(&user_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(db_err)?;
            }
        } else {
            sqlx::query("INSERT OR IGNORE INTO accesses(board_id,user_id) SELECT b.id,u.id FROM boards b JOIN users u ON u.account_id=b.account_id WHERE b.account_id=? AND b.all_access=1 AND u.identity_id=?")
                .bind(&account_id)
                .bind(&identity_id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
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
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok(Html(format!(
        "<!doctype html><html><title>Enter your name</title><main><h1>Great! Now enter your name</h1><form method=post action=\"/{}/users/joins\"><label>Full name <input name=\"user[name]\" autocomplete=name required autofocus></label><button id=log_in>Continue</button></form></main></html>",
        p.slug
    )).into_response())
}
async fn new_user_verification(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok(Html(format!(
        "<!doctype html><html><title>Verify</title><main><form method=post action=\"/{}/users/verifications\"><button>Continue</button></form></main><script>document.forms[0].requestSubmit()</script></html>",
        p.slug
    )).into_response())
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
        json!({"entropy":{"auto_postpone_period_in_days":fields.get("entropy[auto_postpone_period_in_days]")}})
    } else {
        if method == Method::POST {
            return Err(missing());
        }
        serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?
    };
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    p.account_admin()?;
    let Some(value) = nested(&body, "entropy").get("auto_postpone_period_in_days") else {
        return Ok(entropy_parameter_error(if form {
            StatusCode::UNPROCESSABLE_ENTITY
        } else {
            StatusCode::BAD_REQUEST
        }));
    };
    let Some(days) = entropy_days_from_json(value) else {
        return Ok(entropy_parameter_error(StatusCode::INTERNAL_SERVER_ERROR));
    };
    let days = match valid_entropy(days) {
        Ok(days) => days,
        Err(_) => return Ok(entropy_parameter_error(StatusCode::UNPROCESSABLE_ENTITY)),
    };
    let mut tx = begin_write(&app).await.map_err(db_err)?;
    let account_days_sql = if app.mysql {
        "SELECT auto_postpone_period_in_days FROM accounts WHERE id=? FOR UPDATE"
    } else {
        "SELECT auto_postpone_period_in_days FROM accounts WHERE id=?"
    };
    let current_days: i64 = sqlx::query_scalar(account_days_sql)
        .bind(&p.account_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
    if !app.mysql {
        sqlx::query(
            "INSERT INTO card_bulk_touches(board_id) SELECT id FROM boards WHERE account_id=?",
        )
        .bind(&p.account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    }
    sqlx::query("UPDATE accounts SET auto_postpone_period_in_days=? WHERE id=?")
        .bind(days)
        .bind(&p.account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("UPDATE boards SET auto_postpone_period_in_days=? WHERE account_id=? AND entropy_override=0")
        .bind(days).bind(&p.account_id).execute(&mut *tx).await.map_err(db_err)?;
    // The changed-value trigger touches all account cards. Rails also calls
    // touch_all after an unchanged Entropy save, so repeat requests do it here.
    if current_days == days {
        let touch_cards_sql = if app.mysql {
            "UPDATE cards SET updated_at=?,bulk_touch_epoch=bulk_touch_epoch+1 WHERE account_id=?"
        } else {
            "UPDATE cards SET updated_at=? WHERE account_id=?"
        };
        sqlx::query(touch_cards_sql)
            .bind(now())
            .bind(&p.account_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    if !app.mysql {
        sqlx::query("DELETE FROM card_bulk_touches WHERE board_id IN (SELECT id FROM boards WHERE account_id=?)")
            .bind(&p.account_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
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
    if !headers.contains_key("x-rizzy-json-request") {
        return Ok((StatusCode::NOT_ACCEPTABLE, Html(String::new())).into_response());
    }
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let scope =
        " FROM boards b JOIN accesses x ON x.board_id=b.id WHERE b.account_id=? AND x.user_id=?";
    let total: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM access_records WHERE user_id=? AND account_id=?")
            .bind(&p.user_id)
            .bind(&p.account_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
    let rows = sqlx::query(&format!(
        "SELECT b.*{scope} ORDER BY x.accessed_at DESC LIMIT ? OFFSET ?"
    ))
    .bind(&p.account_id)
    .bind(&p.user_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let mut digest = Sha256::new();
    digest.update(b"boards-index\0");
    digest.update(p.account_id.as_bytes());
    digest.update([0]);
    digest.update(p.user_id.as_bytes());
    digest.update(page.to_be_bytes());
    for row in &rows {
        digest.update(row.get::<String, _>("id").as_bytes());
        digest.update([0]);
        digest.update(row.get::<String, _>("updated_at").as_bytes());
        digest.update([0]);
        digest.update(row.get::<String, _>("name").as_bytes());
        digest.update([0]);
        digest.update(row.get::<i64, _>("all_access").to_be_bytes());
        digest.update(
            row.get::<i64, _>("auto_postpone_period_in_days")
                .to_be_bytes(),
        );
        digest.update(row.get::<String, _>("creator_id").as_bytes());
        digest.update([0]);
        if let Some(key) = row.get::<Option<String>, _>("publication_key") {
            digest.update(key.as_bytes());
        }
        digest.update([0]);
        digest.update(row.get::<String, _>("public_description").as_bytes());
        digest.update([0]);
    }
    let etag = format!("W/\"{}\"", hex::encode(&digest.finalize()[..16]));
    let not_modified = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|candidate| candidate.trim() == etag || candidate.trim() == "*")
        });
    let mut response = if not_modified {
        pagination_headers(
            StatusCode::NOT_MODIFIED.into_response(),
            total,
            page,
            &uri,
            &headers,
        )
    } else {
        let out = boards_json_batch(&app, &p, &rows, &base(&headers)).await?;
        paginated(json!(out), total, page, &uri, &headers)
    };
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("hex ETag"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=0, private, must-revalidate"),
    );
    if !not_modified {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
    }
    Ok(response)
}
async fn show_board(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let r = board(&app, &p, &id).await?;
    let mut query = parse_card_query(raw.clone());
    let page = query.page;
    let expanded = raw.as_deref().is_some_and(|raw| {
        url::form_urlencoded::parse(raw.as_bytes())
            .any(|(key, value)| key == "expand_all" && value == "true")
    });
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
    if !headers.contains_key("x-rizzy-json-request") {
        return Ok(if filtered {
            board_page::show_filtered(
                &app,
                &p,
                &r,
                &query,
                raw.as_deref(),
                filter_timezone(&headers),
            )
            .await?
            .into_response()
        } else {
            let key = board_etag_key(&p, &id, page, expanded, &headers);
            if let Some(response) = cached_board_not_modified(&app, &key, &headers).await {
                return Ok(response);
            }
            let before = board_data_version(&app).await;
            let html = board_page::show(&app, &p, &r, page, expanded).await?;
            let response = conditional_html_response(html.0, &headers);
            if let Some(etag) = response
                .headers()
                .get(header::ETAG)
                .and_then(|v| v.to_str().ok())
                .map(str::to_owned)
            {
                remember_board_etag(&app, key, before, etag).await;
            }
            response
        });
    }
    let total: i64 = if filtered {
        let mut count = cards_query(
            "SELECT COUNT(*)",
            &p,
            &query,
            app.mysql,
            filter_timezone(&headers),
        );
        count
            .build_query_scalar()
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?
    } else {
        sqlx::query_scalar("SELECT COUNT(*) FROM cards WHERE board_id=? AND status='published' AND closed=0 AND postponed=0 AND column_id IS NULL")
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?
    };
    let mut etag_salt = String::new();
    if !filtered {
        let (offset, limit) = page_window(page);
        etag_salt.push_str(&format!(
            "page:{page}:board:{}:total:{total}:",
            r.get::<&str, _>("updated_at")
        ));
        etag_salt.push_str(&p.account_updated_at);
        etag_salt.push(':');
        let rows = sqlx::query("SELECT id,updated_at FROM cards WHERE board_id=? AND status='published' AND closed=0 AND postponed=0 AND column_id IS NULL ORDER BY golden DESC,last_active_at DESC,id DESC LIMIT ? OFFSET ?")
            .bind(&id)
            .bind(limit)
            .bind(offset)
            .fetch_all(&app.db)
            .await
            .map_err(db_err)?;
        for row in rows {
            etag_salt.push_str(row.get::<&str, _>("id"));
            etag_salt.push(':');
            etag_salt.push_str(row.get::<&str, _>("updated_at"));
            etag_salt.push(';');
        }
    }
    Ok(pagination_headers(
        conditional_board_json_response(
            board_detail(&app, &p, &r, &base(&headers)).await?,
            &headers,
            &etag_salt,
        ),
        total,
        page,
        &uri,
        &headers,
    ))
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
    if !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
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
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    let form = if content_type.starts_with("application/x-www-form-urlencoded") {
        Some(
            url::form_urlencoded::parse(&body)
                .into_owned()
                .collect::<Vec<_>>(),
        )
    } else {
        None
    };
    let query = url::form_urlencoded::parse(uri.query().unwrap_or("").as_bytes())
        .into_owned()
        .collect::<Vec<_>>();
    if method == Method::POST
        && !form
            .iter()
            .flatten()
            .any(|(key, value)| key == "_method" && value.eq_ignore_ascii_case("put"))
    {
        return Err(missing());
    }
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    board(&app, &p, &id).await?;
    let involvement = if content_type.starts_with("application/json") {
        let body: Value = serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?;
        required(&body, "involvement")?
    } else {
        form.iter()
            .flatten()
            .chain(query.iter())
            .find(|(key, _)| key == "involvement")
            .map(|(_, value)| value.clone())
            .ok_or_else(|| bad("Missing involvement"))?
    };
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
    if headers.contains_key("x-rizzy-json-request") {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let icon_only = form
        .iter()
        .flatten()
        .chain(query.iter())
        .any(|(key, value)| key == "icon_only" && value == "true");
    let label = if involvement == "watching" {
        "Stop watching"
    } else {
        "Watch this"
    };
    let next = if involvement == "watching" {
        "access_only"
    } else {
        "watching"
    };
    let icon = if involvement == "watching" {
        include_str!("../assets/notification-bell.svg")
    } else {
        include_str!("../assets/notification-bell-off.svg")
    };
    let button = format!(
        "<turbo-frame id=\"involvement_button_board_{}\"><form class=\"button_to\" method=\"post\" action=\"/{}/boards/{}/involvement\"><input type=\"hidden\" name=\"_method\" value=\"put\"><input type=\"hidden\" name=\"involvement\" value=\"{next}\"><input type=\"hidden\" name=\"show_watchers\" value=\"false\"><input type=\"hidden\" name=\"icon_only\" value=\"{}\"><button type=\"submit\" class=\"btn{}\"{}><span aria-hidden=\"true\">{icon}</span><span class=\"{}\" id=\"involvement_label_board_{}\">{label}</span></button></form></turbo-frame>",
        escape(&id),
        p.slug,
        escape(&id),
        icon_only,
        if involvement == "watching" && icon_only {
            " btn--reversed"
        } else {
            ""
        },
        if icon_only {
            format!(" title=\"{label}\"")
        } else {
            String::new()
        },
        if icon_only { "for-screen-reader" } else { "" },
        escape(&id),
    );
    Ok(Html(button).into_response())
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
async fn webhook_json(app: &App, p: &Principal, b: &DbRow, w: &DbRow, host: &str) -> Result<Value> {
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
async fn webhook_row(app: &App, p: &Principal, board_id: &str, webhook_id: &str) -> Result<DbRow> {
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
        return Ok(webhook_page::index(
            p.slug,
            &board_id,
            &b.get::<String, _>("name"),
            &rows,
            page,
            total,
            offset,
            limit,
        )
        .into_response());
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
        let deliveries = sqlx::query("SELECT id,state,response,created_at FROM webhook_deliveries WHERE webhook_id=? ORDER BY created_at DESC,id DESC LIMIT 20")
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
    let mut tx = begin_write(&app).await.map_err(db_err)?;
    sqlx::query("INSERT INTO webhooks(id,account_id,board_id,name,url,signing_secret,subscribed_actions,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)")
        .bind(&webhook_id).bind(&p.account_id).bind(&board_id).bind(name).bind(url).bind(webhook_secret())
        .bind(json!(actions).to_string()).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
    sqlx::query("INSERT INTO webhook_delinquency_trackers(id,account_id,webhook_id,created_at,updated_at) VALUES(?,?,?,?,?)")
        .bind(id()).bind(&p.account_id).bind(&webhook_id).bind(&timestamp).bind(&timestamp)
        .execute(&mut *tx).await.map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
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
                "{host}/{account}/cards/{}#comment_{eventable_id}",
                row.get::<i64, _>("number")
            )
        } else {
            format!("{host}/{account}/cards/{}", row.get::<i64, _>("number"))
        };
        let request: Option<Value> = row
            .get::<Option<String>, _>("request")
            .and_then(|s| serde_json::from_str::<Value>(&s).ok())
            .and_then(|stored| {
                stored
                    .get("headers")
                    .and_then(Value::as_object)
                    .map(|headers| {
                        let mut headers = headers.clone();
                        headers.remove("X-Webhook-Signature");
                        json!({"headers": headers})
                    })
            });
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
        .bind(&id).bind(&p.account_id).bind(&p.user_id).bind(name).bind(all as i64).bind(days).bind(requested_days.is_some() as i64).bind(&public_description).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
    rich_text_records::sync(
        &mut tx,
        &p.account_id,
        "Board",
        &id,
        "description",
        "",
        &public_description,
        &timestamp,
    )
    .await?;
    if v.get("public_description").and_then(Value::as_str) == Some("") {
        rich_text_records::ensure(
            &mut tx,
            &p.account_id,
            "Board",
            &id,
            "description",
            &timestamp,
        )
        .await?;
    } else if v.get("public_description") == Some(&Value::Null) {
        rich_text_records::ensure_null(
            &mut tx,
            &p.account_id,
            "Board",
            &id,
            "description",
            &timestamp,
        )
        .await?;
    }
    storage::sync_embeds_tx(&mut tx, &p, "Board", &id, &public_description).await?;
    sqlx::query("INSERT INTO accesses(board_id,user_id,involvement) VALUES(?,?,'watching')")
        .bind(&id)
        .bind(&p.user_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    if all {
        if app.mysql {
            let user_ids: Vec<String> = sqlx::query_scalar(
                "SELECT id FROM users WHERE account_id=? AND active=1 AND role!='system'",
            )
            .bind(&p.account_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(db_err)?;
            for user_id in user_ids {
                sqlx::query("INSERT IGNORE INTO accesses(board_id,user_id) VALUES(?,?)")
                    .bind(&id)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(db_err)?;
            }
        } else {
            sqlx::query("INSERT OR IGNORE INTO accesses(board_id,user_id) SELECT ?,id FROM users WHERE account_id=? AND active=1 AND role!='system'")
                .bind(&id)
                .bind(&p.account_id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
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
    if v.as_object().is_none_or(serde_json::Map::is_empty) {
        return Ok((
            StatusCode::BAD_REQUEST,
            Json(json!({"status":400,"error":"Bad Request"})),
        )
            .into_response());
    }
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
    let public_description = match v.get("public_description") {
        Some(Value::Null) => String::new(),
        _ => string(v, "public_description").unwrap_or_else(|| b.get("public_description")),
    };
    let old_description: String = b.get("public_description");
    let rich_null: Option<i64> = sqlx::query_scalar("SELECT body_is_null FROM rich_text_records WHERE record_type='Board' AND record_id=? AND name='description'")
        .bind(&id).fetch_optional(&app.db).await.map_err(db_err)?;
    let description_changed = match v.get("public_description") {
        Some(Value::Null) => !old_description.is_empty() || rich_null != Some(1),
        Some(Value::String(_)) => old_description != public_description || rich_null != Some(0),
        _ => false,
    };
    let board_changed = name != b.get::<String, _>("name")
        || all != (b.get::<i64, _>("all_access") != 0)
        || days != b.get::<i64, _>("auto_postpone_period_in_days")
        || description_changed;
    let timestamp = if board_changed {
        now()
    } else {
        b.get("updated_at")
    };
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("UPDATE boards SET name=?,all_access=?,auto_postpone_period_in_days=?,entropy_override=?,public_description=?,updated_at=? WHERE id=?")
        .bind(name).bind(all as i64).bind(days).bind(override_days as i64).bind(&public_description).bind(&timestamp).bind(&id).execute(&mut *tx).await.map_err(db_err)?;
    if v.get("public_description") == Some(&Value::Null) {
        rich_text_records::ensure_null(
            &mut tx,
            &p.account_id,
            "Board",
            &id,
            "description",
            &timestamp,
        )
        .await?;
    } else if v
        .get("public_description")
        .and_then(Value::as_str)
        .is_some()
    {
        rich_text_records::sync(
            &mut tx,
            &p.account_id,
            "Board",
            &id,
            "description",
            &old_description,
            &public_description,
            &timestamp,
        )
        .await?;
        rich_text_records::ensure(
            &mut tx,
            &p.account_id,
            "Board",
            &id,
            "description",
            &timestamp,
        )
        .await?;
    }
    let removed_embed_keys =
        storage::sync_embeds_tx(&mut tx, &p, "Board", &id, &public_description).await?;
    if all && b.get::<i64, _>("all_access") == 0 {
        if app.mysql {
            let user_ids: Vec<String> = sqlx::query_scalar(
                "SELECT id FROM users WHERE account_id=? AND active=1 AND role!='system'",
            )
            .bind(&p.account_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(db_err)?;
            for user_id in user_ids {
                sqlx::query("INSERT IGNORE INTO accesses(board_id,user_id) VALUES(?,?)")
                    .bind(&id)
                    .bind(user_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(db_err)?;
            }
        } else {
            sqlx::query("INSERT OR IGNORE INTO accesses(board_id,user_id) SELECT ?,id FROM users WHERE account_id=? AND active=1 AND role!='system'")
                .bind(&id)
                .bind(&p.account_id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
    }
    if let Some(ids) = body
        .get("user_ids")
        .or_else(|| v.get("user_ids"))
        .and_then(Value::as_array)
    {
        let wanted: HashSet<&str> = ids.iter().filter_map(Value::as_str).collect();
        for user in ids.iter().filter_map(Value::as_str) {
            if app.mysql {
                let eligible: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=? AND account_id=? AND active=1 AND role!='system')")
                    .bind(user)
                    .bind(&p.account_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db_err)?;
                if eligible != 0 {
                    sqlx::query("INSERT IGNORE INTO accesses(board_id,user_id) VALUES(?,?)")
                        .bind(&id)
                        .bind(user)
                        .execute(&mut *tx)
                        .await
                        .map_err(db_err)?;
                }
            } else {
                sqlx::query("INSERT OR IGNORE INTO accesses(board_id,user_id) SELECT ?,id FROM users WHERE id=? AND account_id=? AND active=1 AND role!='system'")
                    .bind(&id)
                    .bind(user)
                    .bind(&p.account_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(db_err)?;
            }
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
                clean_inaccessible_board_data(&mut tx, &id, user).await?;
            }
        }
    }
    tx.commit().await.map_err(db_err)?;
    storage::remove_files(removed_embed_keys).await;
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
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let b = board(&app, &p, &id).await?;
    p.admin(&b.get::<String, _>("creator_id"))?;
    let Some(value) = nested(&body, "board").get("auto_postpone_period_in_days") else {
        return Ok(entropy_parameter_error(StatusCode::BAD_REQUEST));
    };
    let Some(days) = entropy_days_from_json(value) else {
        return Ok(entropy_parameter_error(StatusCode::INTERNAL_SERVER_ERROR));
    };
    let days = match valid_entropy(days) {
        Ok(days) => days,
        Err(_) => return Ok(entropy_parameter_error(StatusCode::UNPROCESSABLE_ENTITY)),
    };
    let mut tx = begin_write(&app).await.map_err(db_err)?;
    lock_board_for_write(&mut tx, &id, app.mysql).await?;
    let current = sqlx::query(
        "SELECT auto_postpone_period_in_days,entropy_override FROM boards WHERE id=? AND account_id=?",
    )
    .bind(&id)
    .bind(&p.account_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_err)?;
    let changed = current.get::<i64, _>("entropy_override") == 0
        || current.get::<i64, _>("auto_postpone_period_in_days") != days;
    if !app.mysql {
        sqlx::query("INSERT INTO card_bulk_touches(board_id) VALUES(?)")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    sqlx::query("UPDATE boards SET auto_postpone_period_in_days=?,entropy_override=1 WHERE id=? AND account_id=?")
        .bind(days).bind(&id).bind(&p.account_id).execute(&mut *tx).await.map_err(db_err)?;
    // SQLite's entropy trigger performs the changed-value bulk card touch.
    // Rails also touches cards on an unchanged Entropy save; MySQL needs the
    // explicit touch on every request because its board trigger only stores
    // Entropy metadata. Neither path should touch the board or its columns.
    if app.mysql || !changed {
        let touch_cards_sql = if app.mysql {
            "UPDATE cards SET updated_at=?,bulk_touch_epoch=bulk_touch_epoch+1 WHERE board_id=?"
        } else {
            "UPDATE cards SET updated_at=? WHERE board_id=?"
        };
        sqlx::query(touch_cards_sql)
            .bind(now())
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    if !app.mysql {
        sqlx::query("DELETE FROM card_bulk_touches WHERE board_id=?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    let b = board(&app, &p, &id).await?;
    Ok(Json(board_detail(&app, &p, &b, &base(&headers)).await?).into_response())
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
    let keys = storage::purge_card_storage_tx(
        &mut tx,
        &p.account_id,
        None,
        Some(&id),
        Some(&p.user_id),
        Some(&p.request_id),
    )
    .await?;
    deletion_cleanup::board(&mut tx, &p.account_id, &id).await?;
    sqlx::query("DELETE FROM storage_totals WHERE owner_type='Board' AND owner_id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
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
        let key = publication_key();
        let timestamp = now();
        let mut tx = app.db.begin().await.map_err(db_err)?;
        sqlx::query("INSERT INTO board_publications(id,account_id,board_id,`key`,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(crate::id())
            .bind(&p.account_id)
            .bind(&id)
            .bind(&key)
            .bind(&timestamp)
            .bind(&timestamp)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query("UPDATE boards SET publication_key=?,updated_at=? WHERE id=? AND account_id=?")
            .bind(&key)
            .bind(&timestamp)
            .bind(&id)
            .bind(&p.account_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        tx.commit().await.map_err(db_err)?;
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
    if b.get::<Option<String>, _>("publication_key").is_some() {
        let mut tx = app.db.begin().await.map_err(db_err)?;
        sqlx::query("DELETE FROM board_publications WHERE board_id=? AND account_id=?")
            .bind(&id)
            .bind(&p.account_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query(
            "UPDATE boards SET publication_key=NULL,updated_at=? WHERE id=? AND account_id=?",
        )
        .bind(now())
        .bind(&id)
        .bind(&p.account_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
        tx.commit().await.map_err(db_err)?;
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn published_board(app: &App, account: &str, key: &str) -> Result<DbRow> {
    let account: i64 = account.parse().map_err(|_| missing())?;
    sqlx::query("SELECT b.*,a.name account_name FROM boards b JOIN accounts a ON a.id=b.account_id WHERE a.external_account_id=? AND b.publication_key=?")
        .bind(account).bind(key).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)
}
fn public_page(title: &str, body: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><title>{}</title><style>body{{max-width:48rem;margin:3rem auto;padding:0 1.5rem;font:1.05rem/1.55 system-ui,sans-serif;color:#21211f;background:#faf9f6}}h1{{line-height:1.15}}a{{color:#244789}}li{{padding:.7rem 0;border-bottom:1px solid #ddd}}nav{{margin-bottom:2rem}}article{{white-space:pre-wrap}}</style></head><body>{body}</body></html>",
        escape(title)
    ))
}
async fn checked_column(
    app: &App,
    p: &Principal,
    board_id: &str,
    column_id: &str,
) -> Result<DbRow> {
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
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    board(&app, &p, &board_id).await?;
    if !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let rows = sqlx::query("SELECT * FROM columns WHERE board_id=? ORDER BY position,id")
        .bind(board_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let etag = card_page_etag(&rows);
    if etag_matches(&headers, &etag) {
        return Ok(with_card_page_headers(
            StatusCode::NOT_MODIFIED.into_response(),
            &etag,
        ));
    }
    let mut response = Json(json!(
        rows.iter()
            .map(|r| column_json(&p, r, &base(&headers)))
            .collect::<Vec<_>>()
    ))
    .into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    Ok(with_card_page_headers(response, &etag))
}
fn named_column_count_sql(mysql: bool) -> &'static str {
    if mysql {
        // MySQL otherwise chooses the board-status index and scans cards in
        // other columns before applying column_id to this count.
        "SELECT COUNT(*) FROM cards FORCE INDEX(cards_board_column_active) WHERE board_id=? AND column_id=? AND status='published' AND closed=0 AND postponed=0"
    } else {
        "SELECT COUNT(*) FROM cards WHERE board_id=? AND column_id=? AND status='published' AND closed=0 AND postponed=0"
    }
}

async fn show_column(
    State(app): Shared,
    Path((account, board_id, column_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let c = checked_column(&app, &p, &board_id, &column_id).await?;
    let column_id: String = c.get("id");
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let total: i64 = sqlx::query_scalar(named_column_count_sql(app.mysql))
        .bind(&board_id)
        .bind(&column_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let rows = sqlx::query("SELECT c.*,(SELECT name FROM boards WHERE id=c.board_id) AS board_name,(SELECT name FROM columns WHERE id=c.column_id) AS column_name,(SELECT color FROM columns WHERE id=c.column_id) AS column_color,(SELECT name FROM users WHERE id=c.creator_id) AS creator_name FROM cards c WHERE c.board_id=? AND c.column_id=? AND c.status='published' AND c.closed=0 AND c.postponed=0 ORDER BY c.golden DESC,c.last_active_at DESC,c.id DESC LIMIT ? OFFSET ?")
        .bind(&board_id).bind(&column_id).bind(limit).bind(offset)
        .fetch_all(&app.db).await.map_err(db_err)?;
    let etag = card_page_etag(&rows);
    let json = headers.contains_key("x-rizzy-json-request");
    if etag_matches(&headers, &etag) {
        let response = StatusCode::NOT_MODIFIED.into_response();
        return Ok(with_card_page_headers(
            if json {
                pagination_headers(response, total, page, &uri, &headers)
            } else {
                response
            },
            &etag,
        ));
    }
    let response = if json {
        let mut response = Json(column_json(&p, &c, &base(&headers))).into_response();
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json; charset=utf-8"),
        );
        pagination_headers(response, total, page, &uri, &headers)
    } else {
        let board_name: String = sqlx::query_scalar("SELECT name FROM boards WHERE id=?")
            .bind(&board_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
        let previews = cards_page::previews(&app, &p, &rows).await?;
        named_column_page::render(
            &account,
            &board_id,
            &board_name,
            &column_id,
            &c.get::<String, _>("name"),
            page,
            offset + (rows.len() as i64) < total,
            &previews,
        )
        .into_response()
    };
    Ok(with_card_page_headers(response, &etag))
}
fn column_input(headers: &HeaderMap, body: &[u8]) -> Result<Value> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"));
    if form {
        let mut fields = serde_json::Map::new();
        for (key, value) in url::form_urlencoded::parse(body) {
            let field = key
                .strip_prefix("column[")
                .and_then(|key| key.strip_suffix(']'))
                .unwrap_or(&key);
            if matches!(field, "name" | "color") {
                fields.insert(field.to_owned(), Value::String(value.into_owned()));
            }
        }
        Ok(json!({"column": fields}))
    } else {
        serde_json::from_slice(body).map_err(|_| bad("Invalid JSON"))
    }
}
fn column_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(value) => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        Value::Bool(value) => Some(value.to_string()),
        _ => None,
    }
}
fn column_param_error(headers: &HeaderMap, status: StatusCode) -> Response {
    let (label, html) = if status == StatusCode::BAD_REQUEST {
        ("Bad Request", include_str!("../static/400.html"))
    } else {
        ("Internal Server Error", include_str!("../static/500.html"))
    };
    if headers.contains_key("x-rizzy-json-request") {
        (
            status,
            Json(json!({"status": status.as_u16(), "error": label})),
        )
            .into_response()
    } else {
        (status, Html(html)).into_response()
    }
}
async fn column_stream_markup(
    app: &App,
    p: &Principal,
    board_id: &str,
    c: &DbRow,
) -> Result<String> {
    let id: String = c.get("id");
    let count: i64 = sqlx::query_scalar(named_column_count_sql(app.mysql))
        .bind(board_id)
        .bind(&id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    Ok(board_page::named_column_html(
        p.slug,
        board_id,
        &id,
        &c.get::<String, _>("name"),
        &c.get::<String, _>("color"),
        count,
    ))
}
async fn create_column(
    State(app): Shared,
    Path((account, board_id)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    board(&app, &p, &board_id).await?;
    let body = column_input(&headers, &body)?;
    let v = nested(&body, "column");
    if v.get("name").is_none() && v.get("color").is_none() {
        return Ok(column_param_error(&headers, StatusCode::BAD_REQUEST));
    }
    let Some(name) = v.get("name").and_then(column_scalar) else {
        return Ok(column_param_error(
            &headers,
            StatusCode::INTERNAL_SERVER_ERROR,
        ));
    };
    let color = v
        .get("color")
        .and_then(column_scalar)
        .unwrap_or_else(|| "var(--color-card-default)".into());
    let mut tx = begin_write(&app).await.map_err(db_err)?;
    lock_board_for_write(&mut tx, &board_id, app.mysql).await?;
    let position: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(position),0)+1 FROM columns WHERE board_id=?")
            .bind(&board_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
    let id = id();
    let timestamp = now();
    sqlx::query("INSERT INTO columns(id,account_id,board_id,name,color,position,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
        .bind(&id).bind(&p.account_id).bind(&board_id).bind(name).bind(color).bind(position).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
    sqlx::query("UPDATE boards SET updated_at=? WHERE id=?")
        .bind(&timestamp)
        .bind(&board_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    let c = checked_column(&app, &p, &board_id, &id).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Ok(created(
            format!("/{account}/boards/{board_id}/columns/{id}.json"),
            column_json(&p, &c, &base(&headers)),
        ));
    }
    if wants_turbo_stream(&headers) {
        let content = column_stream_markup(&app, &p, &board_id, &c).await?;
        let mut stream = format!(
            "<turbo-stream action=\"before\" target=\"closed-cards\" method=\"morph\"><template>{content}</template></turbo-stream>"
        );
        let previous = sqlx::query("SELECT * FROM columns WHERE board_id=? AND position<? ORDER BY position DESC,id DESC LIMIT 1")
            .bind(&board_id).bind(position).fetch_optional(&app.db).await.map_err(db_err)?;
        if let Some(previous) = previous {
            let previous_id: String = previous.get("id");
            let content = column_stream_markup(&app, &p, &board_id, &previous).await?;
            stream.push_str(&format!(
                "<turbo-stream action=\"replace\" target=\"column_{}\" method=\"morph\"><template>{content}</template></turbo-stream>",
                escape(&previous_id),
            ));
        }
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            stream,
        )
            .into_response());
    }
    Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"))
}
async fn update_column(
    State(app): Shared,
    Path((account, board_id, column_id)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    checked_column(&app, &p, &board_id, &column_id).await?;
    let body = column_input(&headers, &body)?;
    let v = nested(&body, "column");
    if v.get("name").is_none() && v.get("color").is_none() {
        return Ok(column_param_error(&headers, StatusCode::BAD_REQUEST));
    }
    if v.get("name").is_some_and(Value::is_null) {
        return Ok(column_param_error(
            &headers,
            StatusCode::INTERNAL_SERVER_ERROR,
        ));
    }
    let mut tx = begin_write(&app).await.map_err(db_err)?;
    lock_board_for_write(&mut tx, &board_id, app.mysql).await?;
    let current_sql = if app.mysql {
        "SELECT * FROM columns WHERE id=? AND board_id=? AND account_id=? FOR UPDATE"
    } else {
        "SELECT * FROM columns WHERE id=? AND board_id=? AND account_id=?"
    };
    let c = sqlx::query(current_sql)
        .bind(&column_id)
        .bind(&board_id)
        .bind(&p.account_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)?;
    let name = match v.get("name") {
        None => c.get("name"),
        Some(value) => match column_scalar(value) {
            Some(value) => value,
            None => {
                return Ok(column_param_error(
                    &headers,
                    StatusCode::INTERNAL_SERVER_ERROR,
                ));
            }
        },
    };
    let color = match v.get("color") {
        None => c.get("color"),
        Some(value) => column_scalar(value).unwrap_or_else(|| "var(--color-card-default)".into()),
    };
    if name != c.get::<String, _>("name") || color != c.get::<String, _>("color") {
        let timestamp = now();
        sqlx::query("UPDATE columns SET name=?,color=?,updated_at=? WHERE id=? AND board_id=?")
            .bind(name)
            .bind(color)
            .bind(&timestamp)
            .bind(&column_id)
            .bind(&board_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        let touch_cards_sql = if app.mysql {
            "UPDATE cards SET updated_at=?,bulk_touch_epoch=bulk_touch_epoch+1 WHERE column_id=?"
        } else {
            "UPDATE cards SET updated_at=? WHERE column_id=?"
        };
        sqlx::query(touch_cards_sql)
            .bind(&timestamp)
            .bind(&column_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query("UPDATE boards SET updated_at=? WHERE id=?")
            .bind(&timestamp)
            .bind(&board_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    let c = checked_column(&app, &p, &board_id, &column_id).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Ok(Json(column_json(&p, &c, &base(&headers))).into_response());
    }
    if wants_turbo_stream(&headers) {
        let content = column_stream_markup(&app, &p, &board_id, &c).await?;
        let stream = format!(
            "<turbo-stream action=\"replace\" target=\"column_{}\" method=\"morph\"><template>{content}</template></turbo-stream>",
            escape(&column_id),
        );
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            stream,
        )
            .into_response());
    }
    Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"))
}
async fn delete_column(
    State(app): Shared,
    Path((account, board_id, column_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    checked_column(&app, &p, &board_id, &column_id).await?;
    let mut tx = begin_write(&app).await.map_err(db_err)?;
    lock_board_for_write(&mut tx, &board_id, app.mysql).await?;
    let timestamp = now();
    if app.mysql {
        // Every card on the board receives one timestamp touch. Clear only
        // links to the deleted column in the same scan, avoiding a second
        // write of those rows and a second epoch trigger invocation.
        sqlx::query(
            "UPDATE cards SET column_id=CASE WHEN column_id=? THEN NULL ELSE column_id END,updated_at=?,bulk_touch_epoch=bulk_touch_epoch+1 WHERE board_id=?",
        )
        .bind(&column_id)
        .bind(&timestamp)
        .bind(&board_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    } else {
        sqlx::query("UPDATE cards SET column_id=NULL,bulk_touch_epoch=bulk_touch_epoch+1 WHERE board_id=? AND column_id=?")
            .bind(&board_id)
            .bind(&column_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query(
            "UPDATE cards SET updated_at=?,bulk_touch_epoch=bulk_touch_epoch+1 WHERE board_id=?",
        )
        .bind(&timestamp)
        .bind(&board_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    }
    sqlx::query("DELETE FROM columns WHERE id=? AND board_id=?")
        .bind(&column_id)
        .bind(&board_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    // The bulk epoch suppresses per-card touch callbacks within this write;
    // the board gets one explicit touch below.
    sqlx::query("UPDATE boards SET updated_at=? WHERE id=?")
        .bind(&timestamp)
        .bind(&board_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    if headers.contains_key("x-rizzy-json-request") {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let html = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_none_or(|value| value.contains("text/html") || value.contains("*/*"));
    if html {
        return Ok((
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/boards/{board_id}"))],
        )
            .into_response());
    }
    Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"))
}

async fn move_column(
    app: &App,
    account: &str,
    column_id: &str,
    headers: &HeaderMap,
    left: bool,
) -> Result<Response> {
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

    let mut tx = begin_write(app).await.map_err(db_err)?;
    lock_board_for_write(&mut tx, &board_id, app.mysql).await?;
    let position: i64 = sqlx::query_scalar("SELECT position FROM columns WHERE id=?")
        .bind(column_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)?;
    let neighbor = if left {
        sqlx::query("SELECT id,position FROM columns WHERE board_id=? AND position<? ORDER BY position DESC LIMIT 1")
            .bind(&board_id).bind(position).fetch_optional(&mut *tx).await.map_err(db_err)?
    } else {
        sqlx::query("SELECT id,position FROM columns WHERE board_id=? AND position>? ORDER BY position ASC LIMIT 1")
            .bind(&board_id).bind(position).fetch_optional(&mut *tx).await.map_err(db_err)?
    };
    let neighbor_id = if let Some(neighbor) = neighbor {
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
            .bind(&neighbor_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        Some(neighbor_id)
    } else {
        None
    };
    tx.commit().await.map_err(db_err)?;
    if headers.contains_key("x-rizzy-json-request") {
        return Ok(StatusCode::CREATED.into_response());
    }
    if wants_turbo_stream(headers) {
        let mut stream = String::new();
        if let Some(neighbor_id) = neighbor_id {
            let column = checked_column(app, &p, &board_id, column_id).await?;
            let content = column_stream_markup(app, &p, &board_id, &column).await?;
            let action = if left { "before" } else { "after" };
            stream.push_str(&format!(
                "<turbo-stream action=\"remove\" target=\"column_{}\"></turbo-stream><turbo-stream action=\"{action}\" target=\"column_{}\"><template>{content}</template></turbo-stream>",
                escape(column_id),
                escape(&neighbor_id),
            ));
            let moved_position: i64 = column.get("position");
            for query in [
                "SELECT * FROM columns WHERE board_id=? AND position<? ORDER BY position DESC,id DESC LIMIT 1",
                "SELECT * FROM columns WHERE board_id=? AND position>? ORDER BY position ASC,id ASC LIMIT 1",
            ] {
                let adjacent = sqlx::query(query)
                    .bind(&board_id)
                    .bind(moved_position)
                    .fetch_optional(&app.db)
                    .await
                    .map_err(db_err)?;
                if let Some(adjacent) = adjacent {
                    let adjacent_id: String = adjacent.get("id");
                    let content = column_stream_markup(app, &p, &board_id, &adjacent).await?;
                    stream.push_str(&format!(
                        "<turbo-stream action=\"replace\" target=\"column_{}\" method=\"morph\"><template>{content}</template></turbo-stream>",
                        escape(&adjacent_id),
                    ));
                }
            }
        }
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            stream,
        )
            .into_response());
    }
    Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"))
}

async fn move_column_left(
    State(app): Shared,
    Path((account, column_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    move_column(&app, &account, &column_id, &headers, true).await
}

async fn move_column_right(
    State(app): Shared,
    Path((account, column_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
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
fn card_sort_order(query: &CardQuery) -> &'static str {
    match query.sorted_by.as_deref() {
        Some("oldest") => "c.created_at ASC,c.id ASC",
        Some("newest") => "c.created_at DESC,c.id DESC",
        _ => "c.last_active_at DESC,c.id DESC",
    }
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
fn filter_timezone(headers: &HeaderMap) -> chrono_tz::Tz {
    etag_timezone(headers)
        .and_then(|name| timezone::resolve(&name))
        .unwrap_or(chrono_tz::UTC)
}
fn time_window(input: &str, timezone: chrono_tz::Tz) -> Option<(String, String)> {
    time_window_at(input, timezone, Utc::now())
}
fn local_day_start(date: chrono::NaiveDate, timezone: chrono_tz::Tz) -> Option<String> {
    let midnight = date.and_hms_opt(0, 0, 0)?;
    for minute in 0..=24 * 60 {
        let local = midnight.checked_add_signed(chrono::Duration::minutes(minute))?;
        if let Some(start) = chrono::TimeZone::from_local_datetime(&timezone, &local).latest() {
            return Some(
                start
                    .with_timezone(&Utc)
                    .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
            );
        }
    }
    None
}
fn time_window_at(
    input: &str,
    timezone: chrono_tz::Tz,
    now: chrono::DateTime<Utc>,
) -> Option<(String, String)> {
    let key = input.to_ascii_lowercase().replace([' ', '_', '-'], "");
    let today = now.with_timezone(&timezone).date_naive();
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
    Some((
        local_day_start(start, timezone)?,
        local_day_start(end, timezone)?,
    ))
}
#[cfg(test)]
mod time_window_tests {
    use super::time_window_at;
    use chrono::{DateTime, Utc};

    #[test]
    fn day_boundaries_match_pinned_fizzy_on_dst_and_skipped_dates() {
        // UTC boundaries observed from the pinned TimeWindowParser under Time.use_zone.
        for (zone, at, name, start, end) in [
            (
                chrono_tz::America::New_York,
                "2026-03-08T16:00:00Z",
                "today",
                "2026-03-08T05:00:00.000Z",
                "2026-03-09T04:00:00.000Z",
            ),
            (
                chrono_tz::America::New_York,
                "2026-11-01T17:00:00Z",
                "today",
                "2026-11-01T04:00:00.000Z",
                "2026-11-02T05:00:00.000Z",
            ),
            (
                chrono_tz::America::Santiago,
                "2026-09-06T15:00:00Z",
                "today",
                "2026-09-06T04:00:00.000Z",
                "2026-09-07T03:00:00.000Z",
            ),
            (
                chrono_tz::Pacific::Apia,
                "2011-12-29T22:00:00Z",
                "today",
                "2011-12-29T10:00:00.000Z",
                "2011-12-30T10:00:00.000Z",
            ),
            (
                chrono_tz::Pacific::Apia,
                "2011-12-29T22:00:00Z",
                "this month",
                "2011-12-01T10:00:00.000Z",
                "2011-12-31T10:00:00.000Z",
            ),
            (
                chrono_tz::America::Havana,
                "2026-03-08T16:00:00Z",
                "today",
                "2026-03-08T05:00:00.000Z",
                "2026-03-09T04:00:00.000Z",
            ),
            (
                chrono_tz::America::Havana,
                "2026-11-01T17:00:00Z",
                "today",
                "2026-11-01T05:00:00.000Z",
                "2026-11-02T05:00:00.000Z",
            ),
        ] {
            let now: DateTime<Utc> = at.parse().unwrap();
            assert_eq!(
                time_window_at(name, zone, now),
                Some((start.to_owned(), end.to_owned())),
                "{zone} {at} {name}"
            );
        }
    }
}
fn cards_query<'a>(
    select: &str,
    p: &'a Principal,
    q: &'a CardQuery,
    mysql: bool,
    timezone: chrono_tz::Tz,
) -> QueryBuilder<'a, Db> {
    let mut builder = QueryBuilder::new(format!(
        "{select} FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.account_id="
    ));
    builder.push_bind(&p.account_id).push(" AND c.status='published' AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=").push_bind(&p.user_id).push(")");
    if q.indexed_by.as_deref() != Some("closed")
        && q.closure
            .as_deref()
            .and_then(|value| time_window(value, timezone))
            .is_none()
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
            if mysql {
                builder.push(" AND c.last_active_at>CONCAT(LEFT(DATE_FORMAT(DATE_SUB(UTC_TIMESTAMP(3), INTERVAL b.auto_postpone_period_in_days DAY),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z') AND c.last_active_at<=CONCAT(LEFT(DATE_FORMAT(DATE_SUB(UTC_TIMESTAMP(3), INTERVAL (b.auto_postpone_period_in_days*18) HOUR),'%Y-%m-%dT%H:%i:%s.%f'),23),'Z')");
            } else {
                builder.push(" AND julianday(c.last_active_at)>julianday('now')-b.auto_postpone_period_in_days AND julianday(c.last_active_at)<=julianday('now')-b.auto_postpone_period_in_days*0.75");
            }
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
    if let Some((start, end)) = q
        .creation
        .as_deref()
        .and_then(|value| time_window(value, timezone))
    {
        builder
            .push(" AND c.created_at>=")
            .push_bind(start)
            .push(" AND c.created_at<")
            .push_bind(end);
    }
    if let Some((start, end)) = q
        .closure
        .as_deref()
        .and_then(|value| time_window(value, timezone))
    {
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
        if mysql {
            if let Some(search) = mysql_search::query(&q.terms.join(" "), &p.account_id) {
                let table = mysql_search::table(&p.account_id);
                builder
                    .push(format!(" AND EXISTS (SELECT 1 FROM `{table}` s WHERE s.card_id=c.id AND MATCH(s.account_key,s.content,s.title) AGAINST("))
                    .push_bind(search)
                    .push(" IN BOOLEAN MODE))");
            } else {
                builder.push(" AND 1=0");
            }
        } else {
            let searches: Option<Vec<String>> =
                q.terms.iter().map(|term| fts_query(term)).collect();
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
    if !headers.contains_key("x-rizzy-json-request") {
        return Ok((StatusCode::NOT_ACCEPTABLE, Html(String::new())).into_response());
    }
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
struct SearchQuery {
    q: Option<String>,
    page: Option<i64>,
}
fn parse_search_query(raw: Option<String>) -> SearchQuery {
    let mut query = SearchQuery {
        q: None,
        page: None,
    };
    if let Some(raw) = raw {
        for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
            match key.as_ref() {
                "q" => query.q = Some(value.into_owned()),
                "page" => query.page = value.parse().ok(),
                _ => {}
            }
        }
    }
    query
}
fn search_card_number(term: &str) -> Option<i64> {
    // ActiveRecord casts the search string to an integer for its card-number
    // lookup before running full-text search. Ruby accepts a leading decimal
    // run (including separators) even when other search syntax follows it.
    let mut chars = term.trim_start().chars().peekable();
    let negative = match chars.peek() {
        Some('+') => {
            chars.next();
            false
        }
        Some('-') => {
            chars.next();
            true
        }
        _ => false,
    };
    let mut number = 0_i64;
    let mut saw_digit = false;
    for ch in chars {
        if let Some(digit) = ch.to_digit(10).filter(|_| ch.is_ascii_digit()) {
            saw_digit = true;
            number = number.checked_mul(10)?.checked_add(i64::from(digit))?;
        } else if ch == '_' && saw_digit {
            continue;
        } else {
            break;
        }
    }
    (saw_digit && !negative && number > 0).then_some(number)
}
fn fts_query(raw: &str) -> Option<String> {
    // Match Search::Query's sanitization and ActiveSearch's SQLite term parser.
    // A quote inside a word stays inside that term; it does not start a phrase.
    let balanced = raw.chars().filter(|ch| *ch == '"').count() % 2 == 0;
    let sanitized: String = raw
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || ch == '_' || (balanced && ch == '"') {
                ch
            } else {
                ' '
            }
        })
        .collect();
    let mut tokens = Vec::new();
    let mut remaining = sanitized.trim_start();
    while !remaining.trim().is_empty() {
        if let Some(after_open) = remaining.strip_prefix('"') {
            if let Some(end) = after_open.find('"') {
                tokens.push(format!("\"{}\"", &after_open[..end]));
                remaining = after_open[end + 1..].trim_start();
                continue;
            }
        }
        let end = remaining
            .find(char::is_whitespace)
            .unwrap_or(remaining.len());
        let term = &remaining[..end];
        remaining = remaining[end..].trim_start();
        if term
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
        {
            tokens.push(term.to_owned());
        } else {
            tokens.push(format!("\"{}\"", term.replace('"', "\"\"")));
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
    RawQuery(raw): RawQuery,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let query = parse_search_query(raw);
    if !headers.contains_key("x-rizzy-json-request") {
        return search_page::show(&app, &p, &query).await;
    }
    let page = query.page.unwrap_or(1).clamp(1, 102);
    let Some(term) = query.q.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(paginated(json!([]), 0, page, &uri, &headers));
    };
    let (offset, limit) = page_window(page);
    let number = search_card_number(term);
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
    if app.mysql {
        let Some(search) = mysql_search::query(term, &p.account_id) else {
            return Ok(paginated(json!([]), 0, page, &uri, &headers));
        };
        let table = mysql_search::table(&p.account_id);
        let filter = format!(
            " FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.account_id=? AND c.status='published' AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?) AND EXISTS (SELECT 1 FROM `{table}` s WHERE s.card_id=c.id AND MATCH(s.account_key,s.content,s.title) AGAINST(? IN BOOLEAN MODE))"
        );
        let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*){filter}"))
            .bind(&p.account_id)
            .bind(&p.user_id)
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
        .bind(limit)
        .bind(offset)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
        let items = cards_json_batch(&app, &p, &rows, &base(&headers)).await?;
        return Ok(paginated(json!(items), total, page, &uri, &headers));
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
                "creator_ids" | "creator_ids[]" => query.creators.push(value.into_owned()),
                "board_ids" | "board_ids[]" => query.boards.push(value.into_owned()),
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
) -> QueryBuilder<'a, Db> {
    let mut builder = QueryBuilder::new(format!(
        "{select} FROM events e JOIN cards c ON c.id=e.card_id JOIN boards b ON b.id=e.board_id WHERE e.account_id="
    ));
    builder.push_bind(&p.account_id).push(" AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=")
        .push_bind(&p.user_id).push(") AND e.action IN ('card_assigned','card_auto_postponed','card_board_changed','card_closed','card_postponed','card_published','card_reopened','card_sent_back_to_triage','card_title_changed','card_triaged','card_unassigned','comment_created') AND (e.eventable_type='Card' OR EXISTS (SELECT 1 FROM comments m WHERE m.id=e.eventable_id AND m.card_id=c.id))");
    for (field, values) in [("e.creator_id", &q.creators), ("e.board_id", &q.boards)] {
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
async fn rows_for_ids(app: &App, prefix: &str, ids: &[String]) -> Result<Vec<DbRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder = QueryBuilder::<Db>::new(prefix);
    let mut separated = builder.separated(",");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(")");
    builder.build().fetch_all(&app.db).await.map_err(db_err)
}
async fn assignment_rows_for_cards(app: &App, ids: &[String]) -> Result<Vec<DbRow>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut builder =
        QueryBuilder::<Db>::new("SELECT card_id,assignee_id FROM assignments WHERE card_id IN (");
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
fn event_api_particulars(action: &str, raw: &str) -> Value {
    let stored: Value = serde_json::from_str(raw).unwrap_or(json!({}));
    let nested = stored
        .get("particulars")
        .filter(|value| value.is_object())
        .unwrap_or(&stored);
    let field = |name: &str| nested.get(name).and_then(Value::as_str).unwrap_or("");
    match action {
        "card_assigned" | "card_unassigned" => {
            let ids = match stored.get("assignee_ids") {
                Some(Value::Array(ids)) => ids.clone(),
                Some(Value::Null) | None => Vec::new(),
                Some(id) => vec![id.clone()],
            };
            json!({"assignee_ids":ids})
        }
        "card_board_changed" => {
            json!({"old_board":field("old_board"),"new_board":field("new_board")})
        }
        "card_title_changed" => {
            json!({"old_title":field("old_title"),"new_title":field("new_title")})
        }
        "card_triaged" => json!({"column":field("column")}),
        _ => json!({}),
    }
}
fn event_assignee_names(ids: &[Value], users: &HashMap<String, Value>) -> String {
    let mut names: Vec<(&str, &str)> = ids
        .iter()
        .filter_map(|id| id.as_str())
        .filter_map(|id| {
            users
                .get(id)
                .and_then(|user| user["name"].as_str())
                .map(|name| (id, name))
        })
        .collect();
    names.sort_by_key(|(id, _)| *id);
    let names: Vec<String> = names.into_iter().map(|(_, name)| escape(name)).collect();
    match names.as_slice() {
        [] => String::new(),
        [name] => name.clone(),
        [first, last] => format!("{first} and {last}"),
        _ => format!(
            "{}, and {}",
            names[..names.len() - 1].join(", "),
            names.last().unwrap()
        ),
    }
}
fn activity_description(
    viewer_id: Option<&str>,
    event: &DbRow,
    title: &str,
    particulars: &Value,
    users: &HashMap<String, Value>,
) -> String {
    let actor = escape(
        users
            .get(&event.get::<String, _>("creator_id"))
            .and_then(|u| u["name"].as_str())
            .unwrap_or(""),
    );
    let quoted = format!("&quot;{}&quot;", escape(title));
    let action: String = event.get("action");
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
            if viewer_id.is_some_and(|viewer| {
                ids.is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(viewer)))
            }) {
                format!("{actor} will handle {quoted}")
            } else {
                let names = ids
                    .map(|ids| event_assignee_names(ids, users))
                    .unwrap_or_default();
                format!("{actor} assigned {names} to {quoted}")
            }
        }
        "card_unassigned" => {
            let ids = particulars["assignee_ids"].as_array();
            let target = if viewer_id.is_some_and(|viewer| {
                ids.is_some_and(|ids| ids.iter().any(|id| id.as_str() == Some(viewer)))
            }) {
                "yourself".to_string()
            } else {
                ids.map(|ids| event_assignee_names(ids, users))
                    .unwrap_or_default()
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
    events: &[DbRow],
    host: &str,
    viewer_id: Option<&str>,
) -> Result<Vec<Value>> {
    if events.is_empty() {
        return Ok(Vec::new());
    }
    let particulars: Vec<Value> = events
        .iter()
        .map(|event| {
            event_api_particulars(
                &event.get::<String, _>("action"),
                &event.get::<String, _>("particulars"),
            )
        })
        .collect();
    let card_ids = unique_ids(events.iter().map(|e| e.get::<String, _>("card_id")));
    let card_rows = rows_for_ids(app, "SELECT * FROM cards WHERE id IN (", &card_ids).await?;
    let empty_description_ids = unique_ids(
        card_rows
            .iter()
            .filter(|c| c.get::<String, _>("description").is_empty())
            .map(|c| c.get::<String, _>("id")),
    );
    let present_descriptions =
        rich_text_records::present_ids(&app.db, "Card", "description", &empty_description_ids)
            .await?;
    let board_ids = unique_ids(
        card_rows
            .iter()
            .map(|c| c.get::<String, _>("board_id"))
            .chain(events.iter().map(|e| e.get::<String, _>("board_id"))),
    );
    let board_rows = rows_for_ids(app, "SELECT * FROM boards WHERE id IN (", &board_ids).await?;
    let empty_board_description_ids = unique_ids(
        board_rows
            .iter()
            .filter(|b| {
                b.get::<Option<String>, _>("publication_key").is_some()
                    && b.get::<String, _>("public_description").is_empty()
            })
            .map(|b| b.get::<String, _>("id")),
    );
    let present_board_descriptions = rich_text_records::present_ids(
        &app.db,
        "Board",
        "description",
        &empty_board_description_ids,
    )
    .await?;
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
    let empty_comment_body_ids = unique_ids(
        comment_rows
            .iter()
            .filter(|c| c.get::<String, _>("body").is_empty())
            .map(|c| c.get::<String, _>("id")),
    );
    let present_comment_bodies =
        rich_text_records::present_ids(&app.db, "Comment", "body", &empty_comment_body_ids).await?;
    let image_values = storage::image_urls_for_cards(app, p, &card_ids).await?;
    let embed_owners = storage::embed_owners_for_cards(app, &card_ids).await?;
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
            .chain(events.iter().map(|e| e.get::<String, _>("creator_id")))
            .chain(particulars.iter().flat_map(|value| {
                value["assignee_ids"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|id| id.as_str().map(str::to_owned))
            })),
    );
    let user_rows=rows_for_ids(app,"SELECT u.*,i.email_address FROM users u LEFT JOIN identities i ON i.id=u.identity_id WHERE u.id IN (",&user_ids).await?;
    let users: HashMap<String, Value> = user_rows
        .iter()
        .map(|u| (u.get::<String, _>("id"), user_json_row(p, u, host)))
        .collect();
    let mut boards = HashMap::<String, Value>::new();
    for b in &board_rows {
        let creator = b.get::<String, _>("creator_id");
        let user = users.get(&creator).cloned().ok_or_else(missing)?;
        boards.insert(
            b.get("id"),
            board_json_row(
                p,
                b,
                host,
                user,
                present_board_descriptions.contains(&b.get::<String, _>("id")),
            ),
        );
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
        let (description_plain, mut description_html, rendered_attachments) =
            storage::render_rich_text(app, &p.account_id, p.slug, &description).await?;
        let has_attachments = rendered_attachments || embed_owners.contains(&id);
        if description.is_empty() && !present_descriptions.contains(&id) {
            description_html.clear();
        }
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
            "golden":c.get::<i64,_>("golden")!=0,"last_active_at":json_millisecond_time(c.get::<String,_>("last_active_at")),
            "created_at":json_millisecond_time(c.get::<String,_>("created_at")),"url":format!("{host}/{}/cards/{number}",p.slug),
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
        let (body_plain, mut body_html, _) =
            storage::render_rich_text(app, &p.account_id, p.slug, &body).await?;
        if body.is_empty() && !present_comment_bodies.contains(&id) {
            body_html.clear();
        }
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
            json!({"id":id,"created_at":json_millisecond_time(created),"updated_at":json_millisecond_time(updated),
            "body":{"plain_text":body_plain,"html":body_html},"creator":creator,
            "card":{"id":card_id,"url":format!("{host}/{}/cards/{number}",p.slug)},
            "reactions_url":format!("{host}/{}/cards/{number}/comments/{id}/reactions",p.slug),
            "url":format!("{host}/{}/cards/{number}/comments/{id}",p.slug)}),
        );
    }
    let mut items = Vec::with_capacity(events.len());
    for (e, particulars) in events.iter().zip(particulars.iter()) {
        let card_id: String = e.get("card_id");
        let (number, title, _) = card_meta.get(&card_id).ok_or_else(missing)?;
        let board_id: String = e.get("board_id");
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
        items.push(
            json!({"id":e.get::<String,_>("id"),"action":e.get::<String,_>("action"),
            "created_at":json_millisecond_time(e.get::<String,_>("created_at")),
            "description":activity_description(viewer_id,e,title,particulars,&users),"particulars":particulars,
            "url":url,"eventable_type":eventable_type,"eventable":eventable,
            "board":boards.get(&board_id).cloned().ok_or_else(missing)?,
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
        let next = if watching { "access_only" } else { "watching" };
        let button = format!(
            "<turbo-frame id=\"involvement_button_board_{}\"><form class=\"button_to\" method=\"post\" action=\"/{}/boards/{}/involvement\"><input type=\"hidden\" name=\"_method\" value=\"put\"><input type=\"hidden\" name=\"show_watchers\" value=\"false\"><input type=\"hidden\" name=\"icon_only\" value=\"true\"><input type=\"hidden\" name=\"involvement\" value=\"{next}\"><button type=\"submit\" class=\"watch-button btn{}{}\" data-board-involvement data-url=\"/{}/boards/{}/involvement\" data-involvement=\"{}\" aria-label=\"{}\" title=\"{}\" aria-pressed=\"{}\"><span class=\"watch-icon\" aria-hidden=\"true\">{}{}</span><span class=\"for-screen-reader\">{action}</span></button></form></turbo-frame>",
            escape(&id),
            p.slug,
            escape(&id),
            if watching { " btn--reversed" } else { "" },
            if watching { " is-watching" } else { "" },
            p.slug,
            escape(&id),
            involvement,
            action,
            action,
            watching,
            include_str!("../assets/notification-bell.svg"),
            include_str!("../assets/notification-bell-off.svg"),
        );
        board_html.push_str(&format!(
            "<turbo-frame id=\"involvement_board_{}\"><div class=\"board-involvement flex align-center gap\"><h2 class=\"txt-medium overflow-ellipsis\">{}</h2><hr class=\"board-separator separator--horizontal flex-item-grow\" aria-hidden=\"true\"><label class=\"flex-item-no-shrink txt-small\">{button}</label></div></turbo-frame>",
            escape(&id),
            escape(&name),
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
    let subscription_path = format!(
        "/{}/users/{}/push_subscriptions",
        p.slug,
        escape(&p.user_id)
    );
    let push_attributes = push_delivery::public_key()
        .map(|key| {
            format!(
                " data-push-settings data-vapid-key=\"{}\" data-subscriptions-url=\"{}\"",
                escape(&key),
                subscription_path
            )
        })
        .unwrap_or_default();
    let push_icon = include_str!("../assets/notification-bell.svg");
    let content = format!(
        "<section class=\"settings notification-settings margin-block-start-half\"><div class=\"settings__panel settings__panel--users panel shadow\"><section class=\"settings__section\"><h2 class=\"divider\">Boards</h2><div class=\"settings__scrollable-list flex flex-column gap-half\">{board_html}</div><p id=\"board-involvement-error\" role=\"status\"></p></section></div><div class=\"settings__panel panel shadow\"><section class=\"notifications__status settings__section panel fill-shade center hide-on-native\" data-controller=\"notifications\" data-notifications-subscriptions-url-value=\"{subscription_path}\" data-notifications-enabled-class=\"notifications--on\"{push_attributes}><heading><h2 class=\"txt-medium\">Push notifications are <span class=\"notifications__on-message\">ON</span><span class=\"notifications__off-message\" data-push-state>OFF</span></h2></heading><div class=\"margin-block-start-half\" data-notifications-target=\"subscribeButton\" hidden><button type=\"button\"><span aria-hidden=\"true\">{push_icon}</span><span>Turn ON push notifications on this device</span></button></div><p data-push-error role=\"status\"></p><details class=\"margin-block-start-half\" data-notifications-target=\"explainer\"><summary class=\"btn txt-x-small\"><span class=\"notifications__off-message\">Help me fix this</span><span class=\"notifications__on-message\">Not receiving notifications?</span></summary><div class=\"notifications-help__explainer fill-white margin-block-start border-radius border txt-align-start\"><p>Check that notifications are allowed for this website in your browser and system settings.</p></div></details></section><section class=\"settings__section\"><heading><h2 class=\"divider\">Email Notifications</h2><div>Get a single email with all your notifications every few hours, daily, or weekly.</div></heading><form method=\"post\" action=\"/{}/notifications/settings\" data-controller=\"form\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><strong><label for=\"bundle-email-frequency\">Email me about new notifications...</label></strong><select id=\"bundle-email-frequency\" name=\"user_settings[bundle_email_frequency]\" class=\"input input--select txt-align-center\" data-action=\"change->form#submit\" onchange=\"this.form.requestSubmit()\">{choices}</select></form></section></div></section><script>{}</script><script>{}</script>",
        p.slug,
        include_str!("../assets/notifications_boards.js"),
        include_str!("../assets/notifications_push.js")
    );
    Ok(signed_in_shell_html(p.slug, "Notification Settings", &content, "", include_str!("../static/notification_settings.css"), "<h1 class=\"header__title\" data-bridge--title-target=\"header\">Notification Settings</h1>").into_response())
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
    let statement = if app.mysql {
        "INSERT INTO notification_settings(user_id,account_id,bundle_email_frequency) VALUES(?,?,?) ON DUPLICATE KEY UPDATE bundle_email_frequency=VALUES(bundle_email_frequency)"
    } else {
        "INSERT INTO notification_settings(user_id,account_id,bundle_email_frequency) VALUES(?,?,?) ON CONFLICT(user_id) DO UPDATE SET bundle_email_frequency=excluded.bundle_email_frequency WHERE notification_settings.bundle_email_frequency<>excluded.bundle_email_frequency"
    };
    sqlx::query(statement)
        .bind(&p.user_id)
        .bind(&p.account_id)
        .bind(&frequency)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
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

async fn notification_json(app: &App, p: &Principal, row: &DbRow, host: &str) -> Result<Value> {
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
    rows: Vec<DbRow>,
    host: &str,
) -> Result<Vec<Value>> {
    if rows.is_empty() {
        return Ok(Vec::new());
    }
    let card_ids = unique_ids(rows.iter().map(|row| row.get::<String, _>("card_id")));
    let creator_ids = unique_ids(rows.iter().map(|row| row.get::<String, _>("creator_id")));
    let event_ids = unique_ids(
        rows.iter()
            .filter(|row| row.get::<String, _>("source_type") == "Event")
            .map(|row| row.get::<String, _>("source_id")),
    );
    let mention_ids = unique_ids(
        rows.iter()
            .filter(|row| row.get::<String, _>("source_type") == "Mention")
            .map(|row| row.get::<String, _>("source_id")),
    );
    let cards: HashMap<String, DbRow> = rows_for_ids(
        app,
        "SELECT c.*,b.name board_name FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.id IN (",
        &card_ids,
    )
    .await?
    .into_iter()
    .map(|row| (row.get("id"), row))
    .collect();
    let users: HashMap<String, Value> = rows_for_ids(
        app,
        "SELECT u.*,i.email_address FROM users u LEFT JOIN identities i ON i.id=u.identity_id WHERE u.id IN (",
        &creator_ids,
    )
    .await?
    .into_iter()
    .filter(|row| row.get::<String, _>("account_id") == p.account_id)
    .map(|row| (row.get("id"), user_json_row(p, &row, host)))
    .collect();
    let column_ids = unique_ids(
        cards
            .values()
            .filter_map(|card| card.get::<Option<String>, _>("column_id")),
    );
    let columns: HashMap<String, Value> =
        rows_for_ids(app, "SELECT * FROM columns WHERE id IN (", &column_ids)
            .await?
            .into_iter()
            .map(|row| (row.get("id"), column_json(p, &row, host)))
            .collect();
    let events: HashMap<String, DbRow> =
        rows_for_ids(app, "SELECT * FROM events WHERE id IN (", &event_ids)
            .await?
            .into_iter()
            .map(|row| (row.get("id"), row))
            .collect();
    let mentions: HashMap<String, DbRow> =
        rows_for_ids(app, "SELECT * FROM mentions WHERE id IN (", &mention_ids)
            .await?
            .into_iter()
            .map(|row| (row.get("id"), row))
            .collect();
    let comment_ids = unique_ids(
        events
            .values()
            .filter(|event| event.get::<String, _>("action") == "comment_created")
            .map(|event| event.get::<String, _>("eventable_id"))
            .chain(
                mentions
                    .values()
                    .filter(|mention| mention.get::<String, _>("source_type") == "Comment")
                    .map(|mention| mention.get::<String, _>("source_id")),
            ),
    );
    let comments: HashMap<String, String> = rows_for_ids(
        app,
        "SELECT id,body FROM comments WHERE id IN (",
        &comment_ids,
    )
    .await?
    .into_iter()
    .map(|row| (row.get("id"), row.get("body")))
    .collect();
    let assignee_ids = unique_ids(events.values().filter_map(|event| {
        let particulars: Value =
            serde_json::from_str(&event.get::<String, _>("particulars")).ok()?;
        particulars["assignee_ids"]
            .as_array()?
            .first()?
            .as_str()
            .map(str::to_owned)
    }));
    let assignees: HashMap<String, String> = rows_for_ids(
        app,
        "SELECT id,name,account_id FROM users WHERE id IN (",
        &assignee_ids,
    )
    .await?
    .into_iter()
    .filter(|row| row.get::<String, _>("account_id") == p.account_id)
    .map(|row| (row.get("id"), row.get("name")))
    .collect();
    let mut result = Vec::with_capacity(rows.len());
    for row in rows {
        let card_id: String = row.get("card_id");
        let card = cards.get(&card_id).ok_or_else(missing)?;
        let creator_id: String = row.get("creator_id");
        let creator = users.get(&creator_id).ok_or_else(missing)?;
        let column = card
            .get::<Option<String>, _>("column_id")
            .and_then(|id| columns.get(&id).cloned());
        let raw_title: String = card.get("title");
        let number: i64 = card.get("number");
        let card_title = if raw_title.trim().is_empty() {
            format!("Card {number}")
        } else {
            raw_title.clone()
        };
        let name = creator["name"].as_str().unwrap_or("");
        let source_id: String = row.get("source_id");
        let source_kind: String = row.get("source_type");
        let (source_type, title, body) = match source_kind.as_str() {
            "Mention" => {
                let mention = mentions.get(&source_id).ok_or_else(missing)?;
                let source_body = match mention.get::<String, _>("source_type").as_str() {
                    "Card" => card.get::<String, _>("description"),
                    "Comment" => comments
                        .get(&mention.get::<String, _>("source_id"))
                        .cloned()
                        .ok_or_else(missing)?,
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
            }
            "Event" => {
                let event = events.get(&source_id).ok_or_else(missing)?;
                let action: String = event.get("action");
                let particulars: Value =
                    serde_json::from_str(&event.get::<String, _>("particulars"))
                        .unwrap_or(json!({}));
                let body = match action.as_str() {
                    "card_assigned" => {
                        let assignee = particulars["assignee_ids"]
                            .as_array()
                            .and_then(|ids| ids.first())
                            .and_then(Value::as_str);
                        assignee
                            .map(|id| {
                                format!(
                                    "Assigned to {}",
                                    assignees.get(id).cloned().unwrap_or_else(|| "self".into())
                                )
                            })
                            .unwrap_or_else(|| "Assigned to self".into())
                    }
                    "card_published" => format!("Added by {name}"),
                    "card_closed" => format!("Moved to Done by {name}"),
                    "card_reopened" => format!("Reopened by {name}"),
                    "card_postponed" => format!("Moved to Not Now by {name}"),
                    "card_unassigned" => format!("Unassigned by {name}"),
                    "card_board_changed" => format!("Moved by {name}"),
                    "card_sent_back_to_triage" => format!("Moved back to Maybe? by {name}"),
                    "comment_created" => comments
                        .get(&event.get::<String, _>("eventable_id"))
                        .cloned()
                        .unwrap_or_default()
                        .chars()
                        .take(200)
                        .collect(),
                    _ => name.to_owned(),
                };
                let title = if action == "comment_created" {
                    format!("RE: {card_title}")
                } else {
                    card_title
                };
                ("event", title, body)
            }
            _ => {
                return Err(err(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "Unsupported notification source",
                ));
            }
        };
        let read_at: Option<String> = row.get("read_at");
        result.push(json!({
            "id":row.get::<String,_>("id"),"unread_count":row.get::<i64,_>("unread_count"),
            "read":read_at.is_some(),"read_at":read_at,"created_at":row.get::<String,_>("created_at"),
            "source_type":source_type,"title":title,"body":body,"creator":creator,
            "card":{"id":card_id,"number":number,"title":raw_title,
                "status":card.get::<String,_>("status"),"board_name":card.get::<String,_>("board_name"),
                "closed":card.get::<i64,_>("closed")!=0,"postponed":card.get::<i64,_>("postponed")!=0,
                "url":format!("{host}/{}/cards/{number}",p.slug),"column":column},
            "url":format!("{host}/{}/notifications/{}",p.slug,row.get::<String,_>("id"))
        }));
    }
    Ok(result)
}

async fn notification_page_version(app: &App, p: &Principal) -> Result<(i64, Option<String>, i64)> {
    let statement = if app.mysql {
        "SELECT COUNT(*),MAX(updated_at),CAST(COALESCE(SUM(unread_count),0) AS SIGNED) FROM notifications WHERE user_id=? AND account_id=?"
    } else {
        "SELECT COUNT(*),MAX(updated_at),COALESCE(SUM(unread_count),0) FROM notifications WHERE user_id=? AND account_id=?"
    };
    sqlx::query_as(statement)
        .bind(&p.user_id)
        .bind(&p.account_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)
}

async fn list_notifications(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let json_request = headers.contains_key("x-rizzy-json-request");
    let cache_key = format!("{}:{}:{}", p.account_id, p.user_id, p.slug);
    let mut _build_guard = None;
    let cache_version = if !json_request && !wants_turbo_stream(&headers) && query.page.is_none() {
        let version = notification_page_version(&app, &p).await?;
        if let Some(cached) = app.notification_html_cache.lock().await.get(&cache_key) {
            if cached.version == version && cached.cached_at.elapsed() < Duration::from_secs(1) {
                return Ok(Html(cached.html.clone()).into_response());
            }
        }
        let build_lock = {
            let mut locks = app.notification_html_build_locks.lock().await;
            if locks.len() > 256 {
                locks.retain(|_, lock| lock.strong_count() > 0);
            }
            if let Some(lock) = locks.get(&cache_key).and_then(Weak::upgrade) {
                lock
            } else {
                let lock = Arc::new(tokio::sync::Mutex::new(()));
                locks.insert(cache_key.clone(), Arc::downgrade(&lock));
                lock
            }
        };
        _build_guard = Some(build_lock.lock_owned().await);
        let version = notification_page_version(&app, &p).await?;
        if let Some(cached) = app.notification_html_cache.lock().await.get(&cache_key) {
            if cached.version == version && cached.cached_at.elapsed() < Duration::from_secs(1) {
                return Ok(Html(cached.html.clone()).into_response());
            }
        }
        Some(version)
    } else {
        None
    };
    let show_unread = query.page.is_none();
    let unread_limit = if json_request { 100 } else { 500 };
    let unread = if show_unread {
        sqlx::query("SELECT * FROM notifications WHERE user_id=? AND account_id=? AND read_at IS NULL ORDER BY updated_at DESC LIMIT ?")
            .bind(&p.user_id).bind(&p.account_id).bind(unread_limit).fetch_all(&app.db).await.map_err(db_err)?
    } else {
        Vec::new()
    };
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let total:i64=sqlx::query_scalar("SELECT COUNT(*) FROM notifications WHERE user_id=? AND account_id=? AND read_at IS NOT NULL")
        .bind(&p.user_id).bind(&p.account_id).fetch_one(&app.db).await.map_err(db_err)?;
    let read=sqlx::query("SELECT * FROM notifications WHERE user_id=? AND account_id=? AND read_at IS NOT NULL ORDER BY read_at DESC,updated_at DESC LIMIT ? OFFSET ?")
        .bind(&p.user_id).bind(&p.account_id).bind(limit).bind(offset).fetch_all(&app.db).await.map_err(db_err)?;
    let host = base(&headers);
    let unread_items = notification_items(&app, &p, unread, &host).await?;
    let read_items = notification_items(&app, &p, read, &host).await?;
    if wants_turbo_stream(&headers) && !json_request {
        if !show_unread {
            let stream = notification_page::next_page_stream(
                p.slug,
                &read_items,
                page,
                offset + limit < total,
            );
            return Ok((
                [(
                    header::CONTENT_TYPE,
                    "text/vnd.turbo-stream.html; charset=utf-8",
                )],
                stream,
            )
                .into_response());
        }
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    if !json_request {
        let unread_total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM notifications WHERE user_id=? AND account_id=? AND read_at IS NULL")
            .bind(&p.user_id).bind(&p.account_id).fetch_one(&app.db).await.map_err(db_err)?;
        let html = notification_page::index(
            p.slug,
            &unread_items,
            &read_items,
            page,
            total,
            offset + limit,
            (unread_total - unread_limit).max(0),
            show_unread,
        );
        if let Some(version) = cache_version {
            let mut cache = app.notification_html_cache.lock().await;
            if cache.len() >= 128 && !cache.contains_key(&cache_key) {
                cache.clear();
            }
            cache.insert(
                cache_key,
                NotificationHtmlCacheEntry {
                    version,
                    html: html.clone(),
                    cached_at: Instant::now(),
                },
            );
        }
        return Ok(Html(html).into_response());
    }
    let mut items = unread_items;
    items.extend(read_items);
    Ok(paginated(json!(items), total, page, &uri, &headers))
}

async fn notification_tray(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    RawQuery(raw_query): RawQuery,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let include_read = raw_query.as_deref().and_then(|raw| {
        url::form_urlencoded::parse(raw.as_bytes())
            .filter(|(key, _)| key == "include_read")
            .last()
            .map(|(_, value)| match value.as_ref() {
                "" => None,
                "0" | "f" | "F" | "false" | "FALSE" | "off" | "OFF" => Some(false),
                _ => Some(true),
            })
            .flatten()
    });
    let json_request = headers.contains_key("x-rizzy-json-request");
    // Fizzy keys this validator on the entire notification collection. Reading an
    // older notification must invalidate the unread-only tray as well.
    let version = notification_page_version(&app, &p).await?;
    let etag = format!(
        "W/\"{}\"",
        hash(&format!(
            "tray:{}:{}:{}:{include_read:?}:{version:?}:{:?}:{}:{}",
            p.account_id,
            p.user_id,
            json_request,
            etag_timezone(&headers),
            include_str!("notification_page.rs"),
            include_str!("../static/notification_cards.css")
        ))
    );
    let not_modified = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag));
    if not_modified {
        let mut response = StatusCode::NOT_MODIFIED.into_response();
        response.headers_mut().insert(
            header::ETAG,
            HeaderValue::from_str(&etag).expect("hex ETag"),
        );
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=0, private, must-revalidate"),
        );
        return Ok(response);
    }
    let unread=sqlx::query("SELECT * FROM notifications WHERE user_id=? AND account_id=? AND read_at IS NULL ORDER BY updated_at DESC LIMIT 100")
        .bind(&p.user_id).bind(&p.account_id).fetch_all(&app.db).await.map_err(db_err)?;
    let host = base(&headers);
    let mut items = notification_items(&app, &p, unread, &host).await?;
    if include_read == Some(true) {
        let read=sqlx::query("SELECT * FROM notifications WHERE user_id=? AND account_id=? AND read_at IS NOT NULL ORDER BY read_at DESC,updated_at DESC LIMIT 100")
            .bind(&p.user_id).bind(&p.account_id).fetch_all(&app.db).await.map_err(db_err)?;
        items.extend(notification_items(&app, &p, read, &host).await?);
    }
    let mut response = if json_request {
        Json(json!(items)).into_response()
    } else {
        Html(notification_page::tray(p.slug, &items)).into_response()
    };
    response.headers_mut().insert(
        header::ETAG,
        HeaderValue::from_str(&etag).expect("hex ETag"),
    );
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=0, private, must-revalidate"),
    );
    Ok(response)
}
async fn set_notification_reading(
    app: &App,
    p: &Principal,
    id: &str,
    headers: &HeaderMap,
    read: bool,
) -> Result<Response> {
    p.write()?;
    let id = id.trim_end_matches(".json");
    let timestamp = now();
    let result = if read {
        sqlx::query("UPDATE notifications SET read_at=?,unread_count=0,updated_at=? WHERE id=? AND user_id=? AND account_id=?")
            .bind(&timestamp).bind(&timestamp).bind(id).bind(&p.user_id).bind(&p.account_id)
            .execute(&app.db).await.map_err(db_err)?
    } else {
        sqlx::query("UPDATE notifications SET read_at=NULL,unread_count=1,updated_at=? WHERE id=? AND user_id=? AND account_id=?")
            .bind(&timestamp).bind(id).bind(&p.user_id).bind(&p.account_id)
            .execute(&app.db).await.map_err(db_err)?
    };
    if result.rows_affected() == 0 {
        return Err(missing());
    }
    if wants_turbo_stream(headers) {
        let row =
            sqlx::query("SELECT * FROM notifications WHERE id=? AND user_id=? AND account_id=?")
                .bind(id)
                .bind(&p.user_id)
                .bind(&p.account_id)
                .fetch_one(&app.db)
                .await
                .map_err(db_err)?;
        let notice = notification_json(app, p, &row, &base(headers)).await?;
        let stream = notification_page::reading_stream(p.slug, &notice, read);
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            stream,
        )
            .into_response());
    }
    if !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
async fn read_notification(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    set_notification_reading(&app, &p, &id, &headers, true).await
}
async fn unread_notification(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    set_notification_reading(&app, &p, &id, &headers, false).await
}
async fn read_all_notifications(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    sqlx::query("UPDATE notifications SET read_at=?,unread_count=0,updated_at=? WHERE user_id=? AND account_id=? AND read_at IS NULL")
        .bind(now()).bind(now()).bind(&p.user_id).bind(&p.account_id).execute(&app.db).await.map_err(db_err)?;
    if headers.contains_key("x-rizzy-json-request") {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let from_tray = raw.as_deref().is_some_and(|raw| {
        url::form_urlencoded::parse(raw.as_bytes()).any(|(key, _)| key == "from_tray")
    });
    if from_tray {
        Ok(StatusCode::OK.into_response())
    } else {
        Ok((
            StatusCode::FOUND,
            [(header::LOCATION, format!("/{account}/notifications"))],
        )
            .into_response())
    }
}

async fn list_activities(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    if !headers.contains_key("x-rizzy-json-request") {
        return Ok((StatusCode::NOT_ACCEPTABLE, Html(String::new())).into_response());
    }
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
    let items = activity_items(&app, &p, &rows, &host, Some(&p.user_id)).await?;
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
    let order = card_sort_order(&query);
    let mut count = cards_query(
        "SELECT COUNT(*)",
        &p,
        &query,
        app.mysql,
        filter_timezone(&headers),
    );
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
    let mut builder = cards_query(select, &p, &query, app.mysql, filter_timezone(&headers));
    builder
        .push(format!(" ORDER BY {order} LIMIT "))
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
    let p = principal(&app, &headers, &account).await?;
    let accepts_stream = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.contains("text/vnd.turbo-stream.html"));
    if !uri.path().ends_with(".turbo_stream") && !accepts_stream {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
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
    let order = card_sort_order(&query);
    let mut count = cards_query(
        "SELECT COUNT(*)",
        &p,
        &query,
        app.mysql,
        filter_timezone(&headers),
    );
    let total: i64 = count
        .build_query_scalar()
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let mut builder = cards_query(
        "SELECT c.*,b.name AS board_name,(SELECT name FROM columns WHERE id=c.column_id) AS column_name,(SELECT color FROM columns WHERE id=c.column_id) AS column_color,(SELECT name FROM users WHERE id=c.creator_id) AS creator_name",
        &p,
        &query,
        app.mysql,
        filter_timezone(&headers),
    );
    builder
        .push(format!(" ORDER BY {order} LIMIT "))
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
const FILTER_BOOKMARK_ICON: &str = r#"<svg aria-hidden="true" viewBox="0 0 52 52" width="17" height="17" fill="currentColor"><path d="m14.5 49.6c-1.4.8-3.3.3-4.1-1.1-.3-.5-.4-1-.4-1.5v-41c0-2.2 1.8-4 4-4h24c2.2 0 4 1.8 4 4v41c0 1.7-1.3 3-3 3s-1 0-1.5-.4l-11.5-6.6zm23.5-4.3v-39.3h-24v39.3l10.8-6.1c.8-.4 1.7-.4 2.5 0l10.8 6.1z" fill-rule="evenodd"/></svg>"#;
fn filter_delete_button(slug: i64, filter_id: &str) -> String {
    format!(
        "<form method=post action=\"/{slug}/filters/{}\"><input type=hidden name=_method value=delete><button class=\"filter-save-button is-saved\" type=submit aria-label=\"Delete custom view\" title=\"Delete custom view\">{FILTER_BOOKMARK_ICON}<span class=\"for-screen-reader\">Delete custom view</span></button></form>",
        escape(filter_id)
    )
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
        "<form method=post action=\"/{slug}/filters\">{inputs}<button class=\"filter-save-button\" type=submit aria-label=\"Save custom view\" title=\"Save custom view\">{FILTER_BOOKMARK_ICON}<span class=\"for-screen-reader\">Save custom view</span></button></form>"
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
    if !wants_turbo_stream(&headers) {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok(if let Some(saved_id) = saved_id {
        filter_turbo(filter_delete_button(p.slug, &saved_id))
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
    let saved_id: String = if app.mysql {
        sqlx::query("INSERT INTO filters(id,account_id,creator_id,fields,params_digest,created_at,updated_at) VALUES(?,?,?,?,?,?,?) ON DUPLICATE KEY UPDATE updated_at=VALUES(updated_at)")
            .bind(&filter_id).bind(&p.account_id).bind(&p.user_id).bind(&fields).bind(&digest)
            .bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
        sqlx::query_scalar("SELECT id FROM filters WHERE creator_id=? AND params_digest=?")
            .bind(&p.user_id)
            .bind(&digest)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?
    } else {
        sqlx::query_scalar("INSERT INTO filters(id,account_id,creator_id,fields,params_digest,created_at,updated_at) VALUES(?,?,?,?,?,?,?) ON CONFLICT(creator_id,params_digest) DO UPDATE SET updated_at=excluded.updated_at RETURNING id")
            .bind(&filter_id).bind(&p.account_id).bind(&p.user_id).bind(&fields).bind(&digest)
            .bind(&timestamp).bind(&timestamp).fetch_one(&mut *tx).await.map_err(db_err)?
    };
    for (key, table, column) in [
        ("tag_ids", "filters_tags", "tag_id"),
        ("board_ids", "boards_filters", "board_id"),
        ("assignee_ids", "assignees_filters", "assignee_id"),
        ("creator_ids", "creators_filters", "creator_id"),
        ("closer_ids", "closers_filters", "closer_id"),
    ] {
        if let Some(ids) = params.get(key).and_then(Value::as_array) {
            let statement = if app.mysql {
                format!("INSERT IGNORE INTO {table}(filter_id,{column}) VALUES(?,?)")
            } else {
                format!("INSERT OR IGNORE INTO {table}(filter_id,{column}) VALUES(?,?)")
            };
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
    if !wants_turbo_stream(&headers) {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok(filter_turbo(filter_delete_button(p.slug, &saved_id)))
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
    if !wants_turbo_stream(&headers) {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
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
    if !headers.contains_key("x-rizzy-json-request")
        && !headers.contains_key("x-rizzy-card-fragment")
    {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let page = query.page.unwrap_or(1).max(1);
    let (offset, limit) = page_window(page);
    let total: i64 = sqlx::query_scalar(named_column_count_sql(app.mysql))
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
    // Fizzy's fresh_when validator is based on this page's cards.
    let etag = card_page_etag(&rows);
    if let Some(response) = cached_card_page(&etag, total, page, &uri, &headers) {
        return Ok(response);
    }
    let out = cards_json_batch(&app, &p, &rows, &base(&headers)).await?;
    let mut response = paginated(json!(out), total, page, &uri, &headers);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    Ok(with_card_page_headers(response, &etag))
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
        let timezone = etag_timezone(&headers).unwrap_or_else(|| "UTC".into());
        return Ok(card_page::show(&app, &p, &c, &timezone)
            .await?
            .into_response());
    }
    Ok(Json(card_json(&app, &p, &c, &base(&headers), true).await?).into_response())
}
async fn edit_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let c = card(&app, &p, &number).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let frame_id = format!("edit_card_{}", c.get::<String, _>("id"));
    if headers
        .get("turbo-frame")
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == frame_id)
    {
        return Ok(Html(format!(
            "<html><head></head><body>{}</body></html>",
            card_page::edit_frame(&p, &c)
        ))
        .into_response());
    }
    Ok(card_page::edit(&p, &c).into_response())
}
async fn next_card_number(
    tx: &mut sqlx::Transaction<'_, Db>,
    account_id: &str,
    mysql: bool,
) -> Result<i64> {
    if mysql {
        sqlx::query("UPDATE accounts SET cards_count=cards_count+1 WHERE id=?")
            .bind(account_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        sqlx::query_scalar("SELECT cards_count FROM accounts WHERE id=?")
            .bind(account_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(db_err)
    } else {
        sqlx::query_scalar(
            "UPDATE accounts SET cards_count=cards_count+1 WHERE id=? RETURNING cards_count",
        )
        .bind(account_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(db_err)
    }
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
    let v = card_params(&body)?;
    let title = card_title_param(v)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Untitled".into());
    let description = card_description_param(v).unwrap_or_default();
    let (plain_description, _, _) =
        storage::render_rich_text(&app, &p.account_id, p.slug, &description).await?;
    let search_text = search_content(plain_description);
    let event_id = id();
    let id = id();
    let clock = Utc::now();
    let timestamp = clock.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let created_at = card_timestamp_param(v, "created_at", true, app.mysql)?
        .unwrap_or_else(|| clock.to_rfc3339_opts(chrono::SecondsFormat::Micros, true));
    let last_active_at = card_timestamp_param(v, "last_active_at", true, app.mysql)?
        .unwrap_or_else(|| created_at.clone());
    let mut deadlock_retries = 0;
    let number = loop {
        let write: Result<i64> = async {
            let _writer = app.write_lock.lock().await;
            let mut tx = begin_accessible_board_write(&app, &p, &board_id).await?;
            let number = next_card_number(&mut tx, &p.account_id, app.mysql).await?;
            sqlx::query("INSERT INTO cards(id,account_id,board_id,creator_id,number,title,description,search_text,created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,?,?,?,?,?,?)")
                .bind(&id).bind(&p.account_id).bind(&board_id).bind(&p.user_id).bind(number).bind(&title).bind(&description).bind(&search_text).bind(&created_at).bind(&timestamp).bind(&last_active_at).execute(&mut *tx).await.map_err(db_err)?;
            rich_text_records::sync(
                &mut tx,
                &p.account_id,
                "Card",
                &id,
                "description",
                "",
                &description,
                &timestamp,
            )
            .await?;
            if v.get("description").and_then(Value::as_str) == Some("") {
                rich_text_records::ensure(
                    &mut tx,
                    &p.account_id,
                    "Card",
                    &id,
                    "description",
                    &timestamp,
                )
                .await?;
            }
            storage::attach_card_image_tx(&mut tx, &p, &id, v.get("image")).await?;
            storage::sync_embeds_tx(&mut tx, &p, "Card", &id, &description).await?;
            sqlx::query("INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,1,?,?)")
                .bind(crate::id())
                .bind(&id)
                .bind(&p.user_id)
                .bind(&timestamp)
                .bind(&timestamp)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            sqlx::query("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,particulars,created_at,updated_at) VALUES(?,?,?,?,?,'Card',?,'card_published','{}',?,?)")
                .bind(&event_id).bind(&p.account_id).bind(&board_id).bind(&id).bind(&p.user_id)
                .bind(&id).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
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
                &app,
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
            create_new_mentions(&app, &mut tx, &p, &board_id, &id, "Card", &id, &description).await?;
            if app.mysql {
                mysql_search::sync_card(&mut tx, &p.account_id, &id)
                    .await
                    .map_err(db_err)?;
            }
            tx.commit().await.map_err(db_err)?;
            Ok(number)
        }
        .await;
        match write {
            Ok(number) => break number,
            Err(error) if app.mysql && error.is_retryable_deadlock() && deadlock_retries < 3 => {
                deadlock_retries += 1;
                tokio::time::sleep(Duration::from_millis(10 * deadlock_retries)).await;
            }
            Err(error) => return Err(error),
        }
    };
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
    let mut deadlock_retries = 0;
    let number = loop {
        let write: Result<i64> = async {
            let _guard = app.write_lock.lock().await;
            let mut tx = begin_accessible_board_write(app, p, board_id).await?;
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
                let number = next_card_number(&mut tx, &p.account_id, app.mysql).await?;
                let card_id = id();
                sqlx::query("INSERT INTO cards(id,account_id,board_id,creator_id,number,title,description,status,created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,'','','drafted',?,?,?)")
                    .bind(&card_id).bind(&p.account_id).bind(board_id).bind(&p.user_id).bind(number)
                    .bind(&timestamp).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
                sqlx::query("INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,1,?,?)")
                    .bind(id())
                    .bind(card_id)
                    .bind(&p.user_id)
                    .bind(&timestamp)
                    .bind(&timestamp)
                    .execute(&mut *tx)
                    .await
                    .map_err(db_err)?;
                number
            };
            tx.commit().await.map_err(db_err)?;
            Ok(number)
        }
        .await;
        match write {
            Ok(number) => break number,
            Err(error) if app.mysql && error.is_retryable_deadlock() && deadlock_retries < 3 => {
                deadlock_retries += 1;
                tokio::time::sleep(Duration::from_millis(10 * deadlock_retries)).await;
            }
            Err(error) => return Err(error),
        }
    };
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
    let image_control = if let Some(url) =
        storage::image_url_for_card(&app, &p, &c.get::<String, _>("id"))
            .await?
            .as_str()
    {
        format!(
            "<div class=\"draft-image\"><img src=\"{}\" alt=\"\"><button id=\"remove-image\" type=\"button\" aria-label=\"Remove background image\">Remove background image</button></div>",
            escape(url)
        )
    } else {
        format!(
            "<form id=\"image-upload-form\" action=\"/{account}/cards/{number}\" method=\"post\" enctype=\"multipart/form-data\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><label>Add background image<input id=\"image-upload\" name=\"card[image]\" type=\"file\" accept=\"image/png,image/jpeg,image/jpg,image/webp\" aria-label=\"Add background image\"></label><noscript><button type=\"submit\">Save background image</button></noscript></form>"
        )
    };
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
.draft-image img{{display:block;width:100%;max-height:360px;object-fit:cover}}#image-upload-form{{display:block;margin:18px 28px}}#image-upload-form input[type=file]{{display:block;margin-top:8px}}
</style></head>
<body><a class="skip" href="#main">Skip to main content</a><header><a href="/">Rizzy</a></header>
<main id="main"><nav><a href="/{account}/boards/{board_id}">← Back to {board_name}</a></nav>
<article class="card-perma card-perma--draft"><div class="card-head">Card number {number}<strong>{board_name}</strong></div>
{image_control}
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
document.querySelectorAll('form[action$="/publish"]').forEach(form => form.addEventListener('submit', async event => {{
  event.preventDefault();
  if (await saveDraft()) form.submit();
}}));
const imageForm = document.getElementById('image-upload-form');
document.getElementById('image-upload')?.addEventListener('change', event => {{ if (event.currentTarget.files?.length) imageForm.requestSubmit(); }});
imageForm?.addEventListener('submit', async event => {{
  event.preventDefault();
  const input = document.getElementById('image-upload');
  if (!input.files?.length || !await saveDraft()) return;
  const data = new FormData(imageForm);
  input.disabled = true;
  try {{
    const response = await fetch(imageForm.action, {{method:'POST',headers:{{Accept:'text/vnd.turbo-stream.html'}},body:data}});
    if (!response.ok) throw new Error('Could not attach image');
    location.reload();
  }} catch (error) {{ input.disabled = false; alert(error.message); }}
}});
document.getElementById('remove-image')?.addEventListener('click', async event => {{
  const button = event.currentTarget;
  button.disabled = true;
  try {{
    if (!await saveDraft()) throw new Error('Could not save draft');
    const response = await fetch('/{account}/cards/{number}/image', {{method:'DELETE',headers:{{Accept:'application/json'}}}});
    if (!response.ok) throw new Error('Could not remove image');
    location.reload();
  }} catch (error) {{ button.disabled = false; alert(error.message); }}
}});
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
    let card_id: String = c.get("id");
    let timestamp = now();
    let mut deadlock_retries = 0;
    let board_id = loop {
        let write: Result<String> = async {
            let (mut tx, board_id) = begin_accessible_card_board_write(
                &app,
                &p,
                &card_id,
                &c.get::<String, _>("board_id"),
            )
            .await?;
            sqlx::query("UPDATE cards SET status='published',title=CASE WHEN trim(title)='' THEN 'Untitled' ELSE title END,created_at=?,updated_at=? WHERE id=?")
                .bind(&timestamp).bind(&timestamp).bind(&card_id).execute(&mut *tx).await.map_err(db_err)?;
            let event_id = id();
            sqlx::query("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,particulars,created_at,updated_at) VALUES(?,?,?,?,?,'Card',?,'card_published','{}',?,?)")
                .bind(&event_id).bind(&p.account_id).bind(&board_id).bind(&card_id).bind(&p.user_id)
                .bind(&card_id).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
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
                &app,
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
                &app,
                &mut tx,
                &p,
                &board_id,
                &card_id,
                "Card",
                &card_id,
                &c.get::<String, _>("description"),
            )
            .await?;
            if app.mysql {
                mysql_search::sync_card(&mut tx, &p.account_id, &card_id)
                    .await
                    .map_err(db_err)?;
            }
            tx.commit().await.map_err(db_err)?;
            Ok(board_id)
        }
        .await;
        match write {
            Ok(board_id) => break board_id,
            Err(error) if app.mysql && error.is_retryable_deadlock() && deadlock_retries < 3 => {
                deadlock_retries += 1;
                tokio::time::sleep(Duration::from_millis(10 * deadlock_retries)).await;
            }
            Err(error) => return Err(error),
        }
    };
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

struct CardUpdateInput {
    body: Value,
    image: Option<storage::PendingUpload>,
}

async fn card_update_input(app: &App, request: Request) -> Result<CardUpdateInput> {
    let method = request.method().clone();
    let content_type = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    if content_type.starts_with("multipart/form-data") {
        let mut multipart = axum::extract::Multipart::from_request(request, &())
            .await
            .map_err(|_| bad("Invalid multipart card update"))?;
        let mut card = serde_json::Map::new();
        let mut image = None;
        let mut override_method = None;
        while let Some(field) = multipart
            .next_field()
            .await
            .map_err(|_| bad("Invalid multipart card update"))?
        {
            let Some(name) = field.name().map(str::to_owned) else {
                continue;
            };
            if name == "_method" {
                override_method = Some(field.text().await.map_err(|_| bad("Invalid card update"))?);
                continue;
            }
            let Some(key) = name
                .strip_prefix("card[")
                .and_then(|key| key.strip_suffix(']'))
            else {
                continue;
            };
            if key == "image" {
                if field.file_name().is_some_and(|name| !name.is_empty()) {
                    image = Some(storage::receive_multipart_image(app, field).await?);
                } else {
                    let value = field.text().await.map_err(|_| bad("Invalid card image"))?;
                    if !value.is_empty() {
                        card.insert(key.to_owned(), json!(value));
                    }
                }
            } else {
                let value = field.text().await.map_err(|_| bad("Invalid card update"))?;
                card.insert(key.to_owned(), json!(value));
            }
        }
        if method == Method::POST && !matches!(override_method.as_deref(), Some("patch" | "put")) {
            return Err(missing());
        }
        return Ok(CardUpdateInput {
            body: json!({"card": card}),
            image,
        });
    }
    let body = axum::body::to_bytes(request.into_body(), 2 * 1024 * 1024)
        .await
        .map_err(|_| bad("Card update is too large"))?;
    if content_type.starts_with("application/x-www-form-urlencoded") {
        let mut card = serde_json::Map::new();
        let mut override_method = None;
        for (key, value) in url::form_urlencoded::parse(&body) {
            if key == "_method" {
                override_method = Some(value.to_string());
                continue;
            }
            if let Some(key) = key
                .strip_prefix("card[")
                .and_then(|key| key.strip_suffix(']'))
            {
                card.insert(key.to_owned(), json!(value));
            }
        }
        if method == Method::POST && !matches!(override_method.as_deref(), Some("patch" | "put")) {
            return Err(missing());
        }
        Ok(CardUpdateInput {
            body: json!({"card": card}),
            image: None,
        })
    } else {
        let body = serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?;
        Ok(CardUpdateInput { body, image: None })
    }
}

async fn update_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    request: Request,
) -> Result<Response> {
    let headers = request.headers().clone();
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let original_title: String = c.get("title");
    let input = card_update_input(&app, request).await?;
    let v = card_params(&input.body)?;
    let requested_title = card_title_param(v);
    let requested_description = card_description_param(v);
    let requested_search_text = if let Some(description) = &requested_description {
        let (plain, _, _) =
            storage::render_rich_text(&app, &p.account_id, p.slug, description).await?;
        Some(search_content(plain))
    } else {
        None
    };
    let requested_created_at = card_timestamp_param(v, "created_at", false, app.mysql)?;
    let custom_last_active_at = card_timestamp_param(v, "last_active_at", false, app.mysql)?;
    let title_change_actor = if requested_title.is_some() {
        Some((
            escape(&actor_name(&app, &p).await?),
            system_user(&app, &p).await?,
        ))
    } else {
        None
    };
    let timestamp = now();
    let mut deadlock_retries = 0;
    let (replaced_image_keys, removed_embed_keys) = loop {
        let write: Result<(Vec<String>, Vec<String>)> = async {
            let _writer = app.write_lock.lock().await;
            // Reserve the SQLite writer before reading attachment state in this transaction.
            // A deferred WAL read cannot be promoted after another process commits a write.
            let (mut tx, current_board_id, locked_card) = if c.get::<String, _>("status") == "published" {
                // A move can invalidate the snapshot above. Lock the current board and
                // card together so the UPDATE triggers never reach a different board
                // while this transaction holds the card row.
                let (tx, board_id, current) = begin_accessible_card_board_write_snapshot(
                    &app,
                    &p,
                    &c.get::<String, _>("id"),
                    &c.get::<String, _>("board_id"),
                )
                .await?;
                (tx, board_id, Some(current))
            } else {
                (
                    begin_write(&app).await.map_err(db_err)?,
                    c.get::<String, _>("board_id"),
                    None,
                )
            };
            if app.mysql {
                // A title event needs an account foreign-key lock. Acquire it
                // before rich-text inserts so card creation (account then
                // rich text) cannot form the opposite lock order.
                sqlx::query("SELECT id FROM accounts WHERE id=? FOR SHARE")
                    .bind(&p.account_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db_err)?;
            }
            // The first card read authorizes this request. Fizzy can save its
            // pre-lock title when a concurrent edit changes it, even if this
            // request only supplies a description. The other absent fields use
            // the locked row, including on a deadlock retry.
            let current = if let Some(current) = locked_card {
                current
            } else {
                let current_sql = if app.mysql {
                    "SELECT title,description,search_text,status,created_at,last_active_at FROM cards WHERE id=? FOR UPDATE"
                } else {
                    "SELECT title,description,search_text,status,created_at,last_active_at FROM cards WHERE id=?"
                };
                sqlx::query(current_sql)
                    .bind(c.get::<String, _>("id"))
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db_err)?
            };
            let locked_title: String = current.get("title");
            let status: String = current.get("status");
            let mut title = requested_title
                .clone()
                .unwrap_or_else(|| original_title.clone());
            if status == "published" && title.trim().is_empty() {
                title = "Untitled".into();
            }
            let description = requested_description.clone()
                .unwrap_or_else(|| current.get("description"));
            let search_text = requested_search_text.clone()
                .unwrap_or_else(|| current.get("search_text"));
            let created_at = requested_created_at.clone()
                .unwrap_or_else(|| current.get("created_at"));
            let last_active_at = custom_last_active_at.clone()
                .unwrap_or_else(|| current.get("last_active_at"));
            let title_change = if status == "published"
                && title != original_title
                && !original_title.is_empty()
            {
                let (who, system) = title_change_actor.as_ref()
                    .ok_or_else(|| err(StatusCode::CONFLICT, "Card changed during write"))?;
                let body = format!(
                    "{who} <strong>changed the title</strong> from “{}” to “{}”.",
                    escape(&original_title),
                    escape(&title)
                );
                let content = search_content(rich_text::plain_text(&body));
                Some((system, body, content))
            } else {
                None
            };
            if let Some(upload) = &input.image {
                storage::store_multipart_image_tx(&mut tx, &p, upload).await?;
            }
            let uploaded_image = input.image.as_ref().map(|upload| json!(upload.signed_id));
            let image_value = uploaded_image.as_ref().or(v.get("image"));
            let image_changed = if let Some(value) = image_value {
                let current: Option<String> = sqlx::query_scalar("SELECT b.signed_id FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.record_type='Card' AND a.name='image' AND a.record_id=?")
                    .bind(c.get::<String, _>("id"))
                    .fetch_optional(&mut *tx).await.map_err(db_err)?;
                match value {
                    Value::Null => current.is_some(),
                    Value::String(signed_id) => current.as_deref() != Some(signed_id),
                    _ => true,
                }
            } else {
                false
            };
            let changed = title != locked_title
                || description != current.get::<String, _>("description")
                || search_text != current.get::<String, _>("search_text")
                || created_at != current.get::<String, _>("created_at")
                || last_active_at != current.get::<String, _>("last_active_at")
                || image_changed;
            if changed {
                sqlx::query(
                    "UPDATE cards SET title=?,description=?,search_text=?,status=?,created_at=?,updated_at=?,last_active_at=? WHERE id=?",
                )
                .bind(&title)
                .bind(&description)
                .bind(&search_text)
                .bind(&status)
                .bind(&created_at)
                .bind(&timestamp)
                .bind(&last_active_at)
                .bind(c.get::<String, _>("id"))
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
            }
            if v.get("description") == Some(&Value::Null) {
                sqlx::query("DELETE FROM rich_text_records WHERE record_type='Card' AND record_id=? AND name='description'")
                    .bind(c.get::<String, _>("id")).execute(&mut *tx).await.map_err(db_err)?;
            } else {
                rich_text_records::sync(
                    &mut tx,
                    &p.account_id,
                    "Card",
                    &c.get::<String, _>("id"),
                    "description",
                    &current.get::<String, _>("description"),
                    &description,
                    &timestamp,
                )
                .await?;
            }
            if v.get("description").and_then(Value::as_str) == Some("") {
                rich_text_records::ensure(
                    &mut tx,
                    &p.account_id,
                    "Card",
                    &c.get::<String, _>("id"),
                    "description",
                    &timestamp,
                )
                .await?;
            }
            let replaced_image_keys =
                storage::attach_card_image_tx(&mut tx, &p, &c.get::<String, _>("id"), image_value).await?;
            let removed_embed_keys =
                storage::sync_embeds_tx(&mut tx, &p, "Card", &c.get::<String, _>("id"), &description)
                    .await?;
            if status == "published" {
                create_new_mentions(
                    &app,
                    &mut tx,
                    &p,
                    &current_board_id,
                    &c.get::<String, _>("id"),
                    "Card",
                    &c.get::<String, _>("id"),
                    &description,
                )
                .await?;
                if !changed {
                    sqlx::query("UPDATE boards SET updated_at=? WHERE id=?")
                        .bind(&timestamp)
                        .bind(&current_board_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(db_err)?;
                }
            }
            if app.mysql && changed {
                mysql_search::sync_card(&mut tx, &p.account_id, &c.get::<String, _>("id"))
                    .await
                    .map_err(db_err)?;
            }
            if let Some((system, body, content)) = &title_change {
                let event_timestamp = now();
                let particulars = json!({"old_title":&original_title,"new_title":&title});
                insert_event_tx(
                    &app,
                    &mut tx,
                    &p,
                    &current_board_id,
                    &c.get::<String, _>("id"),
                    "Card",
                    &c.get::<String, _>("id"),
                    "card_title_changed",
                    &particulars,
                    &event_timestamp,
                )
                .await?;
                insert_system_comment_tx(
                    &app,
                    &mut tx,
                    &p,
                    system,
                    &c.get::<String, _>("id"),
                    body,
                    content,
                    &event_timestamp,
                )
                .await?;
            }
            tx.commit().await.map_err(db_err)?;
            Ok((replaced_image_keys, removed_embed_keys))
        }
        .await;
        match write {
            Ok(result) => break result,
            Err(error) if app.mysql && error.is_retryable_deadlock() && deadlock_retries < 3 => {
                deadlock_retries += 1;
                tokio::time::sleep(Duration::from_millis(10 * deadlock_retries)).await;
            }
            Err(error) => return Err(error),
        }
    };
    if let Some(upload) = input.image {
        upload.keep();
    }
    storage::remove_files(replaced_image_keys).await;
    storage::remove_files(removed_embed_keys).await;
    if let Some(last_active_at) = custom_last_active_at {
        sqlx::query("UPDATE cards SET last_active_at=? WHERE id=?")
            .bind(last_active_at)
            .bind(c.get::<String, _>("id"))
            .execute(&app.db)
            .await
            .map_err(db_err)?;
    }
    let c = card(&app, &p, &number).await?;
    if wants_turbo_stream(&headers) && !headers.contains_key("x-rizzy-json-request") {
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            card_page::update_stream(&app, &p, &c).await?,
        )
            .into_response());
    }
    if !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok(Json(card_json(&app, &p, &c, &base(&headers), true).await?).into_response())
}
async fn delete_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    if p.role != "owner" && p.role != "admin" && p.user_id != c.get::<String, _>("creator_id") {
        return Err(err(StatusCode::FORBIDDEN, "Card administrator required"));
    }
    let card_id: String = c.get("id");
    let (mut tx, board_id) =
        begin_accessible_card_board_write(&app, &p, &card_id, &c.get::<String, _>("board_id"))
            .await?;
    let keys = storage::purge_card_storage_tx(
        &mut tx,
        &p.account_id,
        Some(&card_id),
        None,
        Some(&p.user_id),
        Some(&p.request_id),
    )
    .await?;
    deletion_cleanup::card(&mut tx, &p.account_id, &card_id).await?;
    sqlx::query("DELETE FROM cards WHERE id=?")
        .bind(card_id)
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
            [(header::LOCATION, format!("/{account}/boards/{board_id}"))],
        )
            .into_response()
    })
}

async fn card_form_method(
    state: Shared,
    path: Path<(String, String)>,
    request: Request,
) -> Result<Response> {
    let is_form = request
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"));
    if !is_form {
        let multipart = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("multipart/form-data"));
        return if multipart {
            update_card(state, path, request).await
        } else {
            Err(missing())
        };
    }
    let (parts, body) = request.into_parts();
    let body = axum::body::to_bytes(body, 2 * 1024 * 1024)
        .await
        .map_err(|_| bad("Card form is too large"))?;
    let method = url::form_urlencoded::parse(&body)
        .find(|(key, _)| key == "_method")
        .map(|(_, value)| value.into_owned());
    match method.as_deref() {
        Some("delete") => delete_card(state, path, parts.headers).await,
        Some("patch" | "put") => {
            update_card(state, path, Request::from_parts(parts, Body::from(body))).await
        }
        _ => Err(missing()),
    }
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
    let card_id: String = c.get("id");
    let sql = match (flag, value) {
        ("closed", 1) => {
            "UPDATE cards SET closed=?,postponed=0,updated_at=?,last_active_at=? WHERE id=?"
        }
        ("closed", 0) => "UPDATE cards SET closed=?,updated_at=?,last_active_at=? WHERE id=?",
        ("postponed", 1) => {
            "UPDATE cards SET postponed=?,closed=0,column_id=NULL,updated_at=?,last_active_at=? WHERE id=?"
        }
        ("postponed", 0) => "UPDATE cards SET postponed=?,updated_at=?,last_active_at=? WHERE id=?",
        ("golden", _) => "UPDATE cards SET golden=?,updated_at=? WHERE id=?",
        _ => return Err(bad("invalid flag")),
    };
    let _writer = app.write_lock.lock().await;
    let (mut tx, board_id) =
        begin_card_board_write(app, &card_id, &c.get::<String, _>("board_id")).await?;
    let accessible: Option<String> =
        sqlx::query_scalar("SELECT board_id FROM accesses WHERE board_id=? AND user_id=?")
            .bind(&board_id)
            .bind(&p.user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
    if accessible.is_none() {
        return Err(missing());
    }
    let card_state_sql = if app.mysql {
        "SELECT closed,postponed,golden FROM cards WHERE id=? FOR UPDATE"
    } else {
        "SELECT closed,postponed,golden FROM cards WHERE id=?"
    };
    let card_state = sqlx::query(card_state_sql)
        .bind(&card_id)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
    let was_closed = card_state.get::<i64, _>("closed");
    // The pinned source's second postpone resumes an already postponed card
    // before its cached association check, leaving it active.
    let effective_value =
        if flag == "postponed" && value == 1 && card_state.get::<i64, _>("postponed") != 0 {
            0
        } else {
            value
        };
    if (flag == "golden" && card_state.get::<i64, _>("golden") == value)
        || (flag == "closed" && was_closed == value)
    {
        return Ok(StatusCode::NO_CONTENT);
    }
    let timestamp = now();
    if flag == "golden" {
        sqlx::query(sql)
            .bind(effective_value)
            .bind(&timestamp)
            .bind(c.get::<String, _>("id"))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        if value == 1 {
            sqlx::query("INSERT INTO card_goldnesses(id,account_id,card_id,created_at,updated_at) VALUES(?,?,?,?,?)")
                .bind(uuid::Uuid::now_v7().to_string())
                .bind(&p.account_id)
                .bind(c.get::<String, _>("id"))
                .bind(&timestamp)
                .bind(&timestamp)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        } else {
            sqlx::query("DELETE FROM card_goldnesses WHERE card_id=?")
                .bind(c.get::<String, _>("id"))
                .execute(&mut *tx)
                .await
                .map_err(db_err)?;
        }
    } else {
        sqlx::query(sql)
            .bind(effective_value)
            .bind(&timestamp)
            .bind(&timestamp)
            .bind(c.get::<String, _>("id"))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    if flag == "postponed" && effective_value == 1 {
        sqlx::query("DELETE FROM card_not_nows WHERE card_id=?")
            .bind(c.get::<String, _>("id"))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        sqlx::query("INSERT INTO card_not_nows(id,account_id,card_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(uuid::Uuid::now_v7().to_string())
            .bind(&p.account_id)
            .bind(c.get::<String, _>("id"))
            .bind(&p.user_id)
            .bind(&timestamp)
            .bind(&timestamp)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    } else if (flag == "postponed" && effective_value == 0) || (flag == "closed" && value == 1) {
        sqlx::query("DELETE FROM card_not_nows WHERE card_id=?")
            .bind(c.get::<String, _>("id"))
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    if flag == "closed" && value == 1 {
        sqlx::query("INSERT INTO closures(id,account_id,card_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
            .bind(id())
            .bind(&p.account_id)
            .bind(c.get::<String, _>("id"))
            .bind(&p.user_id)
            .bind(&timestamp)
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
    if flag == "closed" && was_closed != value {
        let action = if value == 1 {
            "card_closed"
        } else {
            "card_reopened"
        };
        record_event(
            app,
            p,
            &board_id,
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
            &board_id,
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
async fn claim_auto_postpone_slot(app: &App, slot: &str) -> Result<bool> {
    let insert = if app.mysql {
        "INSERT IGNORE INTO app_secrets(name,value) VALUES('auto_postpone_hour','0')"
    } else {
        "INSERT OR IGNORE INTO app_secrets(name,value) VALUES('auto_postpone_hour','0')"
    };
    sqlx::query(insert).execute(&app.db).await.map_err(db_err)?;
    let result =
        sqlx::query("UPDATE app_secrets SET value=? WHERE name='auto_postpone_hour' AND value<?")
            .bind(slot)
            .bind(slot)
            .execute(&app.db)
            .await
            .map_err(db_err)?;
    Ok(result.rows_affected() == 1)
}

async fn auto_postpone_all_due(app: &App) -> Result<usize> {
    let as_of = Utc::now();
    let comment_body = "<strong>Moved</strong> to “Not Now” due to inactivity";
    let comment_search = search_content(system_plain_text(comment_body));
    let accounts = sqlx::query("SELECT a.id,u.id AS system_user_id FROM accounts a JOIN users u ON u.account_id=a.id AND u.role='system'")
        .fetch_all(&app.db).await.map_err(db_err)?;
    let mut postponed = 0;
    for account in accounts {
        let account_id: String = account.get("id");
        let system_user_id: String = account.get("system_user_id");
        let boards = sqlx::query(
            "SELECT id,auto_postpone_period_in_days FROM boards WHERE account_id=? ORDER BY id",
        )
        .bind(&account_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
        for board in boards {
            let board_id: String = board.get("id");
            let days: i64 = board.get("auto_postpone_period_in_days");
            let cutoff = (as_of - chrono::Duration::days(days))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            // Fizzy's active scope uses the absence of a Not Now record. Scan
            // both flag values separately so a stale flag cannot hide a due
            // card, while each scan still uses cards_board_active.
            for flagged in [0_i64, 1] {
                loop {
                    let cards = sqlx::query("SELECT c.id FROM cards c WHERE c.board_id=? AND c.status='published' AND c.closed=0 AND c.postponed=? AND c.last_active_at<=? AND NOT EXISTS(SELECT 1 FROM card_not_nows n WHERE n.card_id=c.id) ORDER BY c.last_active_at,c.id LIMIT 100")
                    .bind(&board_id).bind(flagged).bind(&cutoff).fetch_all(&app.db).await.map_err(db_err)?;
                    if cards.is_empty() {
                        break;
                    }
                    for card in cards {
                        let card_id: String = card.get("id");
                        let _writer = app.write_lock.lock().await;
                        let mut tx = begin_write(app).await.map_err(db_err)?;
                        lock_board_for_write(&mut tx, &board_id, app.mysql).await?;
                        let still_due = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM cards c WHERE c.id=? AND c.board_id=? AND c.status='published' AND c.closed=0 AND c.last_active_at<=? AND NOT EXISTS(SELECT 1 FROM card_not_nows n WHERE n.card_id=c.id)")
                        .bind(&card_id).bind(&board_id).bind(&cutoff)
                        .fetch_one(&mut *tx).await.map_err(db_err)? != 0;
                        if !still_due {
                            continue;
                        }
                        let timestamp = now();
                        sqlx::query("UPDATE cards SET postponed=1,closed=0,column_id=NULL,updated_at=?,last_active_at=? WHERE id=?")
                        .bind(&timestamp).bind(&timestamp).bind(&card_id)
                        .execute(&mut *tx).await.map_err(db_err)?;
                        sqlx::query("DELETE FROM card_activity_spikes WHERE card_id=?")
                            .bind(&card_id)
                            .execute(&mut *tx)
                            .await
                            .map_err(db_err)?;
                        sqlx::query("INSERT INTO card_not_nows(id,account_id,card_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)")
                        .bind(id()).bind(&account_id).bind(&card_id).bind(&system_user_id)
                        .bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
                        let event_id = id();
                        sqlx::query("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,particulars,created_at,updated_at) VALUES(?,?,?,?,?,'Card',?,'card_auto_postponed','{}',?,?)")
                        .bind(&event_id).bind(&account_id).bind(&board_id).bind(&card_id)
                        .bind(&system_user_id).bind(&card_id).bind(&timestamp).bind(&timestamp)
                        .execute(&mut *tx).await.map_err(db_err)?;
                        queue_webhooks(
                            &mut tx,
                            &account_id,
                            &board_id,
                            &event_id,
                            "card_auto_postponed",
                            &timestamp,
                        )
                        .await?;
                        let comment_id = id();
                        sqlx::query("INSERT INTO comments(id,card_id,creator_id,body,search_text,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
                        .bind(&comment_id).bind(&card_id).bind(&system_user_id)
                        .bind(comment_body).bind(&comment_search).bind(&timestamp).bind(&timestamp)
                        .execute(&mut *tx).await.map_err(db_err)?;
                        rich_text_records::sync(
                            &mut tx,
                            &account_id,
                            "Comment",
                            &comment_id,
                            "body",
                            "",
                            comment_body,
                            &timestamp,
                        )
                        .await?;
                        if app.mysql {
                            mysql_search::sync_comment(&mut tx, &account_id, &comment_id)
                                .await
                                .map_err(db_err)?;
                        }
                        tx.commit().await.map_err(db_err)?;
                        postponed += 1;
                    }
                }
            }
        }
    }
    Ok(postponed)
}
async fn set_card_reading(
    app: &App,
    p: &Principal,
    number: &str,
    read: bool,
    headers: &HeaderMap,
) -> Result<Response> {
    p.write()?;
    let card = card(app, p, number).await?;
    let card_id: String = card.get("id");
    let timestamp = now();
    let cutoff = (Utc::now() - chrono::Duration::minutes(5))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let (mut tx, board_id) =
        begin_accessible_card_board_write(app, p, &card_id, &card.get::<String, _>("board_id"))
            .await?;
    let notification_id: Option<String> = sqlx::query_scalar(
        "SELECT id FROM notifications WHERE card_id=? AND user_id=? AND account_id=?",
    )
    .bind(&card_id)
    .bind(&p.user_id)
    .bind(&p.account_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db_err)?;
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
    if wants_turbo_stream(headers) {
        if !read {
            return Ok((
                StatusCode::NO_CONTENT,
                [(header::CONTENT_TYPE, "text/plain")],
            )
                .into_response());
        }
        let stream = notification_id.map_or_else(
            || "\n".to_string(),
            |id| {
                format!(
                    "<turbo-stream action=\"remove\" target=\"notification_{}\"></turbo-stream>\n",
                    escape(&id)
                )
            },
        );
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            stream,
        )
            .into_response());
    }
    if !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok(if read {
        StatusCode::CREATED
    } else {
        StatusCode::NO_CONTENT
    }
    .into_response())
}
async fn read_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    set_card_reading(&app, &p, &number, true, &headers).await
}
async fn unread_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    set_card_reading(&app, &p, &number, false, &headers).await
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
    let card_id: String = c.get("id");
    let original_board_id: String = c.get("board_id");
    let column_id = column.trim_end_matches(".json");
    let timestamp = now();
    let (mut tx, board_id) = begin_card_board_write(app, &card_id, &original_board_id).await?;
    if board_id != original_board_id {
        tx.rollback().await.map_err(db_err)?;
        return Err(missing());
    }
    let accessible: Option<String> =
        sqlx::query_scalar("SELECT board_id FROM accesses WHERE board_id=? AND user_id=?")
            .bind(&board_id)
            .bind(&p.user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
    if accessible.is_none() {
        return Err(missing());
    }
    let col = sqlx::query("SELECT * FROM columns WHERE id=? AND board_id=? AND account_id=?")
        .bind(column_id)
        .bind(&board_id)
        .bind(&p.account_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)?;
    sqlx::query("UPDATE cards SET column_id=?,closed=0,postponed=0,updated_at=?,last_active_at=? WHERE id=?")
        .bind(column_id).bind(&timestamp).bind(&timestamp).bind(&card_id).execute(&mut *tx).await.map_err(db_err)?;
    for table in ["closures", "card_not_nows", "card_activity_spikes"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE card_id=?"))
            .bind(&card_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    record_event(
        &app,
        &p,
        &board_id,
        &card_id,
        "Card",
        &card_id,
        "card_triaged",
        json!({"column":col.get::<String,_>("name")}),
    )
    .await?;
    let who = escape(&actor_name(&app, &p).await?);
    system_comment(
        &app,
        &p,
        &card_id,
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
    let card_id: String = c.get("id");
    let timestamp = now();
    let (mut tx, board_id) =
        begin_card_board_write(app, &card_id, &c.get::<String, _>("board_id")).await?;
    let accessible: Option<String> =
        sqlx::query_scalar("SELECT board_id FROM accesses WHERE board_id=? AND user_id=?")
            .bind(&board_id)
            .bind(&p.user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
    if accessible.is_none() {
        return Err(missing());
    }
    sqlx::query("UPDATE cards SET column_id=NULL,closed=0,postponed=0,updated_at=?,last_active_at=? WHERE id=?")
        .bind(&timestamp).bind(&timestamp).bind(&card_id).execute(&mut *tx).await.map_err(db_err)?;
    for table in ["closures", "card_not_nows", "card_activity_spikes"] {
        sqlx::query(&format!("DELETE FROM {table} WHERE card_id=?"))
            .bind(&card_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    record_event(
        &app,
        &p,
        &board_id,
        &card_id,
        "Card",
        &card_id,
        "card_sent_back_to_triage",
        json!({}),
    )
    .await?;
    let who = escape(&actor_name(&app, &p).await?);
    system_comment(
        &app,
        &p,
        &card_id,
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
    let mut old_board_id: String = c.get("board_id");
    let mut old_board = board(&app, &p, &old_board_id).await?;
    let mut new_board = board(&app, &p, &destination).await?;
    let card_id: String = c.get("id");
    let mut transfer_tx = None;
    let mut verified = false;
    for _ in 0..16 {
        let mut tx = begin_write(&app).await.map_err(db_err)?;
        let mut board_ids = [&old_board_id, &destination];
        board_ids.sort();
        for board_id in board_ids {
            lock_board_for_write(&mut tx, board_id, app.mysql).await?;
        }
        let card_board_sql = if app.mysql {
            "SELECT board_id FROM cards WHERE id=? FOR UPDATE"
        } else {
            "SELECT board_id FROM cards WHERE id=?"
        };
        let actual_board: String = sqlx::query_scalar(card_board_sql)
            .bind(&card_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?
            .ok_or_else(missing)?;
        if actual_board != old_board_id {
            tx.rollback().await.map_err(db_err)?;
            old_board_id = actual_board;
            old_board = board(&app, &p, &old_board_id).await?;
            continue;
        }
        for board_id in [&old_board_id, &destination] {
            let accessible: Option<String> =
                sqlx::query_scalar("SELECT board_id FROM accesses WHERE board_id=? AND user_id=?")
                    .bind(board_id)
                    .bind(&p.user_id)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(db_err)?;
            if accessible.is_none() {
                return Err(missing());
            }
        }
        verified = true;
        if old_board_id == destination {
            tx.rollback().await.map_err(db_err)?;
            break;
        }
        old_board = sqlx::query("SELECT * FROM boards WHERE id=?")
            .bind(&old_board_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
        new_board = sqlx::query("SELECT * FROM boards WHERE id=?")
            .bind(&destination)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
        transfer_tx = Some(tx);
        break;
    }
    if !verified {
        return Err(err(StatusCode::CONFLICT, "Card moved during write"));
    }
    if let Some(mut tx) = transfer_tx {
        storage::record_board_transfer_tx(
            &mut tx,
            app.mysql,
            &p.account_id,
            &card_id,
            &old_board_id,
            &destination,
            &p.user_id,
            &p.request_id,
        )
        .await?;
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
            if app.mysql {
                let user_ids: Vec<String> = sqlx::query_scalar("SELECT a.assignee_id FROM assignments a JOIN users u ON u.id=a.assignee_id WHERE a.card_id=? AND u.active=1")
                    .bind(&card_id)
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(db_err)?;
                for user_id in user_ids {
                    sqlx::query("INSERT IGNORE INTO accesses(board_id,user_id,involvement) VALUES(?,?,'access_only')")
                        .bind(&destination)
                        .bind(user_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(db_err)?;
                }
            } else {
                sqlx::query("INSERT OR IGNORE INTO accesses(board_id,user_id,involvement) SELECT ?,a.assignee_id,'access_only' FROM assignments a JOIN users u ON u.id=a.assignee_id WHERE a.card_id=? AND u.active=1")
                    .bind(&destination)
                    .bind(&card_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(db_err)?;
            }
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
        if app.mysql {
            mysql_search::sync_card(&mut tx, &p.account_id, &card_id)
                .await
                .map_err(db_err)?;
            let comment_ids: Vec<String> =
                sqlx::query_scalar("SELECT id FROM comments WHERE card_id=?")
                    .bind(&card_id)
                    .fetch_all(&mut *tx)
                    .await
                    .map_err(db_err)?;
            for comment_id in comment_ids {
                mysql_search::sync_comment(&mut tx, &p.account_id, &comment_id)
                    .await
                    .map_err(db_err)?;
            }
        }
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
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "tag_title is required"))?
    } else {
        let json: Value = serde_json::from_slice(&body).map_err(|_| bad("Invalid JSON"))?;
        string(&json, "tag_title")
            .ok_or_else(|| err(StatusCode::BAD_REQUEST, "tag_title is required"))?
    };
    if tag_title.trim().is_empty() {
        return Err(err(StatusCode::BAD_REQUEST, "tag_title is required"));
    }
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    // Fizzy strips outer whitespace, removes exactly one leading hash, then
    // normalizes the stored title to lowercase. Spaces after the hash remain.
    let stripped = tag_title.trim();
    let title = stripped
        .strip_prefix('#')
        .unwrap_or(stripped)
        .to_lowercase();
    if title.starts_with('#') {
        return Err(bad("Tag title cannot start with #"));
    }
    let card_id: String = c.get("id");
    let (mut tx, _) =
        begin_accessible_card_board_write(&app, &p, &card_id, &c.get::<String, _>("board_id"))
            .await?;
    let tag_id = id();
    let tag_created_at = now();
    let insert = if app.mysql {
        "INSERT IGNORE INTO tags(id,account_id,title,created_at,updated_at) VALUES(?,?,?,?,?)"
    } else {
        "INSERT INTO tags(id,account_id,title,created_at,updated_at) VALUES(?,?,?,?,?) ON CONFLICT(account_id,title) DO NOTHING"
    };
    sqlx::query(insert)
        .bind(&tag_id)
        .bind(&p.account_id)
        .bind(&title)
        .bind(&tag_created_at)
        .bind(&tag_created_at)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    let tag_id: String = sqlx::query_scalar("SELECT id FROM tags WHERE account_id=? AND title=?")
        .bind(&p.account_id)
        .bind(&title)
        .fetch_one(&mut *tx)
        .await
        .map_err(db_err)?;
    let exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM taggings WHERE card_id=? AND tag_id=?")
            .bind(&card_id)
            .bind(&tag_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
    if exists == 0 {
        let timestamp = now();
        sqlx::query(
            "INSERT INTO taggings(id,card_id,tag_id,created_at,updated_at) VALUES(?,?,?,?,?)",
        )
        .bind(id())
        .bind(&card_id)
        .bind(&tag_id)
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    } else {
        sqlx::query("DELETE FROM taggings WHERE card_id=? AND tag_id=?")
            .bind(&card_id)
            .bind(&tag_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    // Fizzy's Tagging touches its card; the card touch also updates the board.
    sqlx::query("UPDATE cards SET updated_at=? WHERE id=?")
        .bind(now())
        .bind(&card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    if wants_turbo_stream(&headers) {
        let tags = tagging_page::tags(&app, &p, &c).await?;
        let picker = tagging_page::picker(&app, &p, &c).await?;
        let stream = format!(
            "<turbo-stream action=\"replace\" method=\"morph\" target=\"tags_card_{}\"><template>{tags}</template></turbo-stream>{picker}",
            escape(&card_id)
        );
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            stream,
        )
            .into_response());
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
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let c = card(&app, &p, &number).await?;
    if !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let card_id: String = c.get("id");
    let rows = sqlx::query("SELECT * FROM steps WHERE card_id=? ORDER BY created_at,id")
        .bind(&card_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let mut digest = Sha256::new();
    digest.update(card_id.as_bytes());
    for row in &rows {
        for field in ["id", "updated_at", "content"] {
            digest.update([0]);
            digest.update(row.get::<String, _>(field).as_bytes());
        }
        digest.update([row.get::<i64, _>("completed") as u8]);
    }
    let etag = format!("W/\"{}\"", hex::encode(&digest.finalize()[..16]));
    if etag_matches(&headers, &etag) {
        return Ok(with_card_page_headers(
            StatusCode::NOT_MODIFIED.into_response(),
            &etag,
        ));
    }
    let items = rows.iter().map(step_value).collect::<Vec<_>>();
    let mut response = with_card_page_headers(Json(items).into_response(), &etag);
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json; charset=utf-8"),
    );
    Ok(response)
}
async fn step_row(app: &App, card_id: &str, step: &str) -> Result<DbRow> {
    sqlx::query("SELECT * FROM steps WHERE id=? AND card_id=?")
        .bind(step)
        .bind(card_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(missing)
}
fn step_value(row: &DbRow) -> Value {
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
fn step_input(headers: &HeaderMap, body: &Bytes) -> Result<Value> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"));
    if form {
        let mut fields = serde_json::Map::new();
        for (key, value) in url::form_urlencoded::parse(body) {
            match key.as_ref() {
                "step[content]" | "content" => {
                    fields.insert("content".into(), json!(value.as_ref()));
                }
                "step[completed]" | "completed" => {
                    fields.insert("completed".into(), json!(value.as_ref()));
                }
                _ => {}
            }
        }
        Ok(json!({"step":fields}))
    } else {
        serde_json::from_slice(body).map_err(|_| bad("Invalid JSON"))
    }
}
fn step_completed(value: Option<&Value>, previous: bool) -> bool {
    match value {
        Some(Value::Bool(value)) => *value,
        Some(Value::String(value)) => value == "1" || value.eq_ignore_ascii_case("true"),
        Some(Value::Number(value)) => value.as_i64() == Some(1),
        _ => previous,
    }
}
fn step_frame(account: &str, number: &str, id: &str, content: &str, completed: bool) -> String {
    let id = escape(id);
    let path = format!("/{account}/cards/{number}/steps/{id}");
    let checked = if completed { " checked" } else { "" };
    format!(
        "<turbo-frame id=\"step_{id}\"><li class=\"step\"><form action=\"{path}\" method=\"post\" data-controller=\"form\"><input type=\"hidden\" name=\"_method\" value=\"put\"><input type=\"hidden\" name=\"step[completed]\" value=\"0\"><input type=\"checkbox\" name=\"step[completed]\" value=\"1\" class=\"checkbox\" data-action=\"change-&gt;form#submit\"{checked}></form><a href=\"{path}/edit\" class=\"step__content\">{}</a></li></turbo-frame>",
        escape(content)
    )
}
fn step_stream(action: &str, target: &str, template: Option<&str>) -> String {
    let target = escape(target);
    match template {
        Some(html) => format!(
            "<turbo-stream action=\"{action}\" target=\"{target}\"><template>{html}</template></turbo-stream>\n"
        ),
        None => format!("<turbo-stream action=\"{action}\" target=\"{target}\"></turbo-stream>\n"),
    }
}
async fn create_step(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let body = step_input(&headers, &body)?;
    let v = nested(&body, "step");
    let content = required(v, "content")?;
    let completed = step_completed(v.get("completed"), false);
    let id = id();
    let timestamp = now();
    let card_id: String = c.get("id");
    let (mut tx, _) =
        begin_accessible_card_board_write(&app, &p, &card_id, &c.get::<String, _>("board_id"))
            .await?;
    sqlx::query(
        "INSERT INTO steps(id,card_id,content,completed,created_at,updated_at) VALUES(?,?,?,?,?,?)",
    )
    .bind(&id)
    .bind(&card_id)
    .bind(&content)
    .bind(completed as i64)
    .bind(&timestamp)
    .bind(&timestamp)
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
    sqlx::query("UPDATE cards SET updated_at=? WHERE id=?")
        .bind(&timestamp)
        .bind(&card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    if wants_turbo_stream(&headers) {
        let card_id: String = c.get("id");
        let html = step_frame(&account, &number, &id, &content, completed);
        let stream = step_stream("before", &format!("new_step_card_{card_id}"), Some(&html));
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            stream,
        )
            .into_response());
    }
    if !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok(created(
        format!("/{account}/cards/{number}/steps/{id}.json"),
        json!({"id":id,"content":content,"completed":completed}),
    ))
}
async fn update_step(
    State(app): Shared,
    Path((account, number, step)): Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
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
    let body = step_input(&headers, &body)?;
    let v = nested(&body, "step");
    let previous_content: String = old.get("content");
    let previous_completed = old.get::<i64, _>("completed") != 0;
    let content = string(v, "content").unwrap_or_else(|| previous_content.clone());
    let completed = step_completed(v.get("completed"), previous_completed);
    let timestamp = now();
    let card_id: String = c.get("id");
    let (mut tx, _) =
        begin_accessible_card_board_write(&app, &p, &card_id, &c.get::<String, _>("board_id"))
            .await?;
    let changed = sqlx::query("UPDATE steps SET content=?,completed=?,updated_at=? WHERE id=? AND card_id=? AND (content<>? OR completed<>?)")
        .bind(&content)
        .bind(completed as i64)
        .bind(&timestamp)
        .bind(&step)
        .bind(&card_id)
        .bind(&content)
        .bind(completed as i64)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?
        .rows_affected()
        != 0;
    if changed {
        sqlx::query("UPDATE cards SET updated_at=? WHERE id=?")
            .bind(&timestamp)
            .bind(&card_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    if wants_turbo_stream(&headers) {
        let card_id: String = c.get("id");
        let html = step_frame(&account, &number, &step, &content, completed);
        let bubble = format!(
            "<div id=\"bubble_card_{}\" hidden class=\"bubble\"></div>",
            escape(&card_id)
        );
        let stream = step_stream("replace", &format!("step_{step}"), Some(&html))
            + &step_stream("replace", &format!("bubble_card_{card_id}"), Some(&bubble));
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            stream,
        )
            .into_response());
    }
    if !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok(Json(json!({"id":step,"content":content,"completed":completed})).into_response())
}
async fn delete_step(
    State(app): Shared,
    Path((account, number, step)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let card_id: String = c.get("id");
    let (mut tx, _) =
        begin_accessible_card_board_write(&app, &p, &card_id, &c.get::<String, _>("board_id"))
            .await?;
    let done = sqlx::query("DELETE FROM steps WHERE id=? AND card_id=?")
        .bind(&step)
        .bind(&card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    if done.rows_affected() == 0 {
        return Err(missing());
    }
    sqlx::query("UPDATE cards SET updated_at=? WHERE id=?")
        .bind(now())
        .bind(&card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    if wants_turbo_stream(&headers) {
        let stream = step_stream("remove", &format!("step_{step}"), None);
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            stream,
        )
            .into_response());
    }
    if !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok(StatusCode::NO_CONTENT.into_response())
}
async fn comment_row(app: &App, card: &DbRow, id: &str) -> Result<DbRow> {
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
    card: &DbRow,
    r: &DbRow,
    host: &str,
) -> Result<Value> {
    let id: String = r.get("id");
    let card_id: String = card.get("id");
    let number: i64 = card.get("number");
    let body: String = r.get("body");
    let created = json_millisecond_time(r.get::<String, _>("created_at"));
    let updated = r
        .get::<Option<String>, _>("updated_at")
        .unwrap_or_else(|| created.clone());
    let creator = user_json(app, p, &r.get::<String, _>("creator_id"), host).await?;
    let (plain, mut html) = if creator["role"] == "system" {
        (system_plain_text(&body), wrap_html(&body))
    } else {
        let (plain, html, _) = storage::render_rich_text(app, &p.account_id, p.slug, &body).await?;
        (plain, html)
    };
    if body.is_empty()
        && rich_text_records::present_ids(&app.db, "Comment", "body", std::slice::from_ref(&id))
            .await?
            .is_empty()
    {
        html.clear();
    }
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
    if !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
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
        let mut comment = json!({});
        if let Some(content) = fields.get("comment[body]") {
            comment["body"] = json!(content);
        }
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
    let body_param =
        if comment_input.get("body").is_none() && comment_input.get("created_at").is_some() {
            None
        } else {
            comment_body_param(comment_input)?
        };
    let content = body_param.clone().unwrap_or_default();
    let (plain_body, _, _) =
        storage::render_rich_text(&app, &p.account_id, p.slug, &content).await?;
    let search_text = search_content(plain_body);
    let id = id();
    let clock = Utc::now();
    let timestamp = clock.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let created_at = timestamp_param(comment_input, "created_at")?
        .unwrap_or_else(|| clock.to_rfc3339_opts(chrono::SecondsFormat::Micros, true));
    let _writer = app.write_lock.lock().await;
    let (mut tx, board_id) = begin_accessible_card_board_write(
        &app,
        &p,
        &c.get::<String, _>("id"),
        &c.get::<String, _>("board_id"),
    )
    .await?;
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
    rich_text_records::sync(
        &mut tx,
        &p.account_id,
        "Comment",
        &id,
        "body",
        "",
        &content,
        &timestamp,
    )
    .await?;
    if body_param.as_deref() == Some("") {
        rich_text_records::ensure(&mut tx, &p.account_id, "Comment", &id, "body", &timestamp)
            .await?;
    }
    storage::sync_embeds_tx(&mut tx, &p, "Comment", &id, &content).await?;
    create_new_mentions(
        &app,
        &mut tx,
        &p,
        &board_id,
        &c.get::<String, _>("id"),
        "Comment",
        &id,
        &content,
    )
    .await?;
    let watch_statement = if app.mysql {
        "INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,1,?,?) ON DUPLICATE KEY UPDATE updated_at=IF(watching<>1,VALUES(updated_at),updated_at),watching=1"
    } else {
        "INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,1,?,?) ON CONFLICT(card_id,user_id) DO UPDATE SET watching=1,updated_at=excluded.updated_at WHERE watches.watching!=1"
    };
    sqlx::query(watch_statement)
        .bind(crate::id())
        .bind(c.get::<String, _>("id"))
        .bind(&p.user_id)
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    if app.mysql {
        mysql_search::sync_comment(&mut tx, &p.account_id, &id)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    record_event(
        &app,
        &p,
        &board_id,
        &c.get::<String, _>("id"),
        "Comment",
        &id,
        "comment_created",
        json!({}),
    )
    .await?;
    let period: i64 =
        sqlx::query_scalar("SELECT auto_postpone_period_in_days FROM boards WHERE id=?")
            .bind(&board_id)
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
        let fresh_form = card_page::new_comment_form(&p, &c, false);
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
    if wants_turbo_stream(&headers) && !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    if !headers.contains_key("x-rizzy-json-request") {
        let frame = comment_edit_page::rendered_comment_frame(&app, &p, &c, &r).await?;
        return Ok(signed_in_frame_page(p.slug, "Comment", frame));
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
    let result = update_comment_data(&app, &account, &number, &id, &headers, &body).await?;
    if wants_turbo_stream(&headers) {
        comment_replace_stream(&app, &account, &number, &id, &headers).await
    } else if headers.contains_key("x-rizzy-json-request") {
        Ok(Json(result).into_response())
    } else {
        Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"))
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
            let result = update_comment_data(
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
            if headers.contains_key("x-rizzy-json-request") {
                return Ok(Json(result).into_response());
            }
            Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"))
        }
        Some("delete") => delete_comment(State(app), Path((account, number, id)), headers).await,
        _ => Err(missing()),
    }
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
    let body_param = comment_body_param(nested(&body, "comment"))?;
    let content = body_param.clone().unwrap_or_default();
    let (plain_body, _, _) =
        storage::render_rich_text(&app, &p.account_id, p.slug, &content).await?;
    let search_text = search_content(plain_body);
    let card_id: String = c.get("id");
    let (mut tx, board_id) =
        begin_accessible_card_board_write(app, &p, &card_id, &c.get::<String, _>("board_id"))
            .await?;
    let old_body: String =
        sqlx::query_scalar("SELECT body FROM comments WHERE id=? AND card_id=? AND creator_id=?")
            .bind(id)
            .bind(&card_id)
            .bind(&p.user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?
            .ok_or_else(missing)?;
    let old_rich: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM rich_text_records WHERE record_type='Comment' AND record_id=? AND name='body' AND body_is_null=0",
    )
    .bind(id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_err)?;
    let changed =
        old_body != content || (old_body.is_empty() && (old_rich > 0) != body_param.is_some());
    if !changed {
        tx.rollback().await.map_err(db_err)?;
        let r = comment_row(app, &c, id).await?;
        return comment_json(app, &p, &c, &r, &base(headers)).await;
    }
    let timestamp = now();
    sqlx::query("UPDATE comments SET body=?,search_text=?,updated_at=? WHERE id=?")
        .bind(&content)
        .bind(&search_text)
        .bind(&timestamp)
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    if body_param.is_some() {
        rich_text_records::sync(
            &mut tx,
            &p.account_id,
            "Comment",
            &id,
            "body",
            &old_body,
            &content,
            &timestamp,
        )
        .await?;
    } else {
        sqlx::query("DELETE FROM rich_text_records WHERE record_type='Comment' AND record_id=? AND name='body'")
            .bind(&id).execute(&mut *tx).await.map_err(db_err)?;
    }
    if body_param.as_deref() == Some("") {
        rich_text_records::ensure(&mut tx, &p.account_id, "Comment", &id, "body", &timestamp)
            .await?;
    }
    let keys = storage::sync_embeds_tx(&mut tx, &p, "Comment", &id, &content).await?;
    create_new_mentions(
        &app, &mut tx, &p, &board_id, &card_id, "Comment", &id, &content,
    )
    .await?;
    if app.mysql {
        mysql_search::sync_comment(&mut tx, &p.account_id, id)
            .await
            .map_err(db_err)?;
    }
    sqlx::query("UPDATE cards SET updated_at=? WHERE id=?")
        .bind(&timestamp)
        .bind(&card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
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
    let card_id: String = c.get("id");
    let (mut tx, _) =
        begin_accessible_card_board_write(&app, &p, &card_id, &c.get::<String, _>("board_id"))
            .await?;
    let keys = storage::purge_record_embeds_tx(&mut tx, &p, "Comment", &comment).await?;
    deletion_cleanup::comment(&mut tx, &p.account_id, &comment).await?;
    sqlx::query(
        "DELETE FROM events WHERE eventable_type='Comment' AND eventable_id=? AND card_id=?",
    )
    .bind(&comment)
    .bind(c.get::<String, _>("id"))
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
    if app.mysql {
        mysql_search::remove(&mut tx, &p.account_id, "Comment", &comment)
            .await
            .map_err(db_err)?;
    }
    sqlx::query("UPDATE cards SET updated_at=? WHERE id=?")
        .bind(now())
        .bind(&card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
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
    match toggle_assignment_to(&app, &p, &c, &assignee_id).await {
        Ok(_) => {}
        Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, _, _)) if wants_turbo_stream(&headers) => {
            return assignment_stream(&app, &p, &c).await;
        }
        Err(error) => return Err(error),
    }
    if wants_turbo_stream(&headers) {
        return assignment_stream(&app, &p, &c).await;
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

async fn toggle_self_assignment(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    match toggle_assignment_to(&app, &p, &c, &p.user_id).await {
        Ok(_) => {}
        Err(ApiError(StatusCode::UNPROCESSABLE_ENTITY, _, _)) if wants_turbo_stream(&headers) => {
            return assignment_stream(&app, &p, &c).await;
        }
        Err(error) => return Err(error),
    }
    if wants_turbo_stream(&headers) {
        assignment_stream(&app, &p, &c).await
    } else if headers.contains_key("x-rizzy-json-request") {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"))
    }
}

async fn assignment_stream(app: &App, p: &Principal, c: &DbRow) -> Result<Response> {
    let card_id: String = c.get("id");
    let meta = assignment_page::meta(app, p, c).await?;
    let picker = assignment_page::picker(app, p, c).await?;
    let stream = format!(
        "<turbo-stream action=\"replace\" method=\"morph\" target=\"meta_card_{}\"><template>{meta}</template></turbo-stream>{picker}",
        escape(&card_id)
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

async fn toggle_assignment_to(
    app: &App,
    p: &Principal,
    c: &DbRow,
    assignee_id: &str,
) -> Result<StatusCode> {
    let card_id: String = c.get("id");
    let who = escape(&actor_name(app, p).await?);
    let (mut tx, board_id) =
        begin_accessible_card_board_write(app, p, &card_id, &c.get::<String, _>("board_id"))
            .await?;
    let assignee=sqlx::query("SELECT u.id,u.name FROM users u JOIN boards b ON b.id=? WHERE u.id=? AND u.account_id=? AND u.active=1 AND u.role!='system' AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=u.id)")
        .bind(&board_id).bind(assignee_id).bind(&p.account_id).fetch_optional(&mut *tx).await.map_err(db_err)?.ok_or_else(missing)?;
    let existed: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM assignments WHERE card_id=? AND assignee_id=?")
            .bind(&card_id)
            .bind(assignee_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
    let target = escape(&assignee.get::<String, _>("name"));
    if existed > 0 {
        sqlx::query("DELETE FROM assignments WHERE card_id=? AND assignee_id=?")
            .bind(&card_id)
            .bind(assignee_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    } else {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM assignments WHERE card_id=?")
            .bind(&card_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_err)?;
        if count >= 100 {
            return Err(err(
                StatusCode::UNPROCESSABLE_ENTITY,
                "Card already has the maximum of 100 assignees",
            ));
        }
        let timestamp = now();
        sqlx::query("INSERT INTO assignments(id,account_id,card_id,assignee_id,assigner_id,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
            .bind(id()).bind(&p.account_id).bind(&card_id).bind(assignee_id).bind(&p.user_id).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
        let timestamp = now();
        let statement = if app.mysql {
            "INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,1,?,?) ON DUPLICATE KEY UPDATE updated_at=IF(watching<>1,VALUES(updated_at),updated_at),watching=1"
        } else {
            "INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,1,?,?) ON CONFLICT(card_id,user_id) DO UPDATE SET watching=1,updated_at=excluded.updated_at WHERE watches.watching!=1"
        };
        sqlx::query(statement)
            .bind(id())
            .bind(&card_id)
            .bind(assignee_id)
            .bind(&timestamp)
            .bind(&timestamp)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    sqlx::query("UPDATE cards SET updated_at=? WHERE id=?")
        .bind(now())
        .bind(&card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    let (action, comment) = if existed > 0 {
        (
            "card_unassigned",
            format!("{who} <strong>unassigned</strong> from {target}."),
        )
    } else {
        (
            "card_assigned",
            format!("{who} <strong>assigned</strong> this to {target}."),
        )
    };
    record_event(
        app,
        p,
        &board_id,
        &card_id,
        "Card",
        &card_id,
        action,
        json!({"assignee_ids":[assignee_id]}),
    )
    .await?;
    system_comment(app, p, &card_id, &comment).await?;
    if existed == 0 {
        record_activity_spike(&app, &p.account_id, &card_id).await?;
    }
    Ok(StatusCode::NO_CONTENT)
}
async fn set_watch(app: &App, p: &Principal, number: &str, watching: bool) -> Result<StatusCode> {
    p.write()?;
    let c = card(app, p, number).await?;
    let card_id: String = c.get("id");
    let timestamp = now();
    let (mut tx, board_id) =
        begin_card_board_write(app, &card_id, &c.get::<String, _>("board_id")).await?;
    let accessible: Option<String> =
        sqlx::query_scalar("SELECT board_id FROM accesses WHERE board_id=? AND user_id=?")
            .bind(&board_id)
            .bind(&p.user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
    if accessible.is_none() {
        return Err(missing());
    }
    let statement = if app.mysql {
        "INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,?,?,?) ON DUPLICATE KEY UPDATE updated_at=IF(watching<>VALUES(watching),VALUES(updated_at),updated_at),watching=VALUES(watching)"
    } else {
        "INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,?,?,?) ON CONFLICT(card_id,user_id) DO UPDATE SET watching=excluded.watching,updated_at=excluded.updated_at WHERE watches.watching!=excluded.watching"
    };
    let result = sqlx::query(statement)
        .bind(id())
        .bind(&card_id)
        .bind(&p.user_id)
        .bind(watching as i64)
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    if result.rows_affected() != 0 {
        sqlx::query("UPDATE cards SET updated_at=? WHERE id=?")
            .bind(&timestamp)
            .bind(&card_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn set_pin(app: &App, p: &Principal, number: &str, pinned: bool) -> Result<StatusCode> {
    p.write()?;
    let c = card(app, p, number).await?;
    let card_id: String = c.get("id");
    let (mut tx, board_id) =
        begin_card_board_write(app, &card_id, &c.get::<String, _>("board_id")).await?;
    let accessible: Option<String> =
        sqlx::query_scalar("SELECT board_id FROM accesses WHERE board_id=? AND user_id=?")
            .bind(&board_id)
            .bind(&p.user_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?;
    if accessible.is_none() {
        return Err(missing());
    }
    if pinned {
        let timestamp = now();
        let statement = if app.mysql {
            "INSERT IGNORE INTO pins(id,account_id,card_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)"
        } else {
            "INSERT OR IGNORE INTO pins(id,account_id,card_id,user_id,created_at,updated_at) VALUES(?,?,?,?,?,?)"
        };
        sqlx::query(statement)
            .bind(id())
            .bind(&p.account_id)
            .bind(&card_id)
            .bind(&p.user_id)
            .bind(&timestamp)
            .bind(&timestamp)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    } else {
        sqlx::query("DELETE FROM pins WHERE card_id=? AND user_id=?")
            .bind(&card_id)
            .bind(&p.user_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn list_pins(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let json_request = headers.contains_key("x-rizzy-json-request");
    let limit = if json_request { 100 } else { 20 };
    let rows=sqlx::query("SELECT c.*,pin.id pin_id,pin.created_at pin_created_at,b.name board_name,(SELECT name FROM columns WHERE id=c.column_id) column_name,(SELECT color FROM columns WHERE id=c.column_id) column_color,(SELECT name FROM users WHERE id=c.creator_id) creator_name FROM pins pin JOIN cards c ON c.id=pin.card_id JOIN boards b ON b.id=c.board_id WHERE pin.user_id=? AND pin.account_id=? AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?) ORDER BY pin.created_at DESC,pin.id DESC LIMIT ?")
        .bind(&p.user_id).bind(&p.account_id).bind(&p.user_id).bind(limit).fetch_all(&app.db).await.map_err(db_err)?;
    if !json_request {
        let version: Vec<(String, String, String)> = rows
            .iter()
            .map(|row| {
                (
                    row.get("pin_id"),
                    row.get("pin_created_at"),
                    row.get("updated_at"),
                )
            })
            .collect();
        let etag = format!(
            "W/\"{}\"",
            hash(&format!(
                "pins:{}:{}:{version:?}:{}:{}",
                p.account_id,
                p.user_id,
                include_str!("pins_page.rs"),
                include_str!("../static/pins.css")
            ))
        );
        let unchanged = headers
            .get(header::IF_NONE_MATCH)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.split(',').any(|candidate| candidate.trim() == etag));
        if unchanged {
            let mut response = StatusCode::NOT_MODIFIED.into_response();
            response.headers_mut().insert(
                header::ETAG,
                HeaderValue::from_str(&etag).expect("pin ETag"),
            );
            response.headers_mut().insert(
                header::CACHE_CONTROL,
                HeaderValue::from_static("max-age=0, private, must-revalidate"),
            );
            return Ok(response);
        }
        let previews = cards_page::preview_fragments(&app, &p, &rows).await?;
        let items: Vec<pins_page::PinPreview> = rows
            .iter()
            .zip(previews)
            .map(|(row, card)| pins_page::PinPreview {
                id: row.get("pin_id"),
                number: row.get("number"),
                card,
            })
            .collect();
        let mut response = Html(pins_page::index(p.slug, &items)).into_response();
        response.headers_mut().insert(
            header::ETAG,
            HeaderValue::from_str(&etag).expect("pin ETag"),
        );
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("max-age=0, private, must-revalidate"),
        );
        return Ok(response);
    }
    let host = base(&headers);
    let mut out = Vec::new();
    for r in &rows {
        out.push(card_json(&app, &p, r, &host, false).await?);
    }
    Ok(Json(json!(out)).into_response())
}

async fn reaction_json(
    app: &App,
    p: &Principal,
    r: &DbRow,
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
    c: &DbRow,
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
fn reaction_body(headers: &HeaderMap, body: &Bytes) -> Result<Value> {
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"))
    {
        let fields: HashMap<String, String> = url::form_urlencoded::parse(body)
            .map(|(key, value)| (key.into_owned(), value.into_owned()))
            .collect();
        Ok(
            json!({"reaction":{"content":fields.get("reaction[content]").or_else(||fields.get("content"))}}),
        )
    } else {
        serde_json::from_slice(body).map_err(|_| bad("Invalid JSON"))
    }
}
async fn create_reaction(
    app: &App,
    p: &Principal,
    c: &DbRow,
    comment: Option<&str>,
    host: &str,
    headers: &HeaderMap,
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
    let card_id: String = c.get("id");
    let (mut tx, _) =
        begin_accessible_card_board_write(app, p, &card_id, &c.get::<String, _>("board_id"))
            .await?;
    let timestamp = now();
    sqlx::query("INSERT INTO reactions(id,account_id,reactable_type,reactable_id,reacter_id,content,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?)")
        .bind(&id).bind(&p.account_id).bind(kind).bind(&target).bind(&p.user_id).bind(content)
        .bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
    if kind == "Comment" {
        sqlx::query("UPDATE comments SET updated_at=? WHERE id=?")
            .bind(now())
            .bind(&target)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    let timestamp = now();
    sqlx::query("UPDATE cards SET last_active_at=?,updated_at=? WHERE id=?")
        .bind(&timestamp)
        .bind(&timestamp)
        .bind(&card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    let r = sqlx::query("SELECT * FROM reactions WHERE id=?")
        .bind(&id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    if wants_turbo_stream(headers) {
        let html = reaction_page::reactions_html(app, p, c, comment).await?;
        let target = escape(&target);
        let kind = kind.to_lowercase();
        let stream = format!(
            "<turbo-stream action=\"replace\" target=\"reacting_{kind}_{target}\"><template>{html}</template></turbo-stream>"
        );
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            stream,
        )
            .into_response());
    }
    if !headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok((
        StatusCode::CREATED,
        Json(reaction_json(app, p, &r, host, c.get("number"), comment).await?),
    )
        .into_response())
}
async fn delete_reaction(
    app: &App,
    p: &Principal,
    c: &DbRow,
    comment: Option<&str>,
    reaction: &str,
    headers: &HeaderMap,
) -> Result<Response> {
    p.write()?;
    let (kind, target) = if let Some(id) = comment {
        let r = comment_row(app, c, id).await?;
        ("Comment", r.get::<String, _>("id"))
    } else {
        ("Card", c.get::<String, _>("id"))
    };
    let card_id: String = c.get("id");
    let (mut tx, _) =
        begin_accessible_card_board_write(app, p, &card_id, &c.get::<String, _>("board_id"))
            .await?;
    let r=sqlx::query("SELECT reacter_id FROM reactions WHERE id=? AND account_id=? AND reactable_type=? AND reactable_id=?")
        .bind(reaction).bind(&p.account_id).bind(kind).bind(&target).fetch_optional(&mut *tx).await.map_err(db_err)?.ok_or_else(missing)?;
    if r.get::<String, _>("reacter_id") != p.user_id {
        return Err(err(StatusCode::FORBIDDEN, "Reaction creator required"));
    }
    sqlx::query("DELETE FROM reactions WHERE id=?")
        .bind(reaction)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    if kind == "Comment" {
        sqlx::query("UPDATE comments SET updated_at=? WHERE id=?")
            .bind(now())
            .bind(&target)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    sqlx::query("UPDATE cards SET updated_at=? WHERE id=?")
        .bind(now())
        .bind(&card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    if wants_turbo_stream(headers) {
        return Ok((
            [(
                header::CONTENT_TYPE,
                "text/vnd.turbo-stream.html; charset=utf-8",
            )],
            format!(
                "<turbo-stream action=\"remove\" target=\"reaction_{}\"></turbo-stream>",
                escape(reaction)
            ),
        )
            .into_response());
    }
    if headers.contains_key("x-rizzy-json-request") {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"))
    }
}
async fn list_card_reactions(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &h, &a).await?;
    let c = card(&app, &p, &n).await?;
    if h.contains_key("x-rizzy-json-request") {
        Ok(Json(reactions_for(&app, &p, &c, None, &base(&h)).await?).into_response())
    } else {
        let frame = reaction_page::reactions_html(&app, &p, &c, None).await?;
        Ok(signed_in_frame_page(p.slug, "Reactions", frame))
    }
}
async fn create_card_reaction(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let p = principal(&app, &h, &a).await?;
    let c = card(&app, &p, &n).await?;
    create_reaction(&app, &p, &c, None, &base(&h), &h, reaction_body(&h, &body)?).await
}
async fn delete_card_reaction(
    State(app): Shared,
    Path((a, n, r)): Path<(String, String, String)>,
    h: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &h, &a).await?;
    let c = card(&app, &p, &n).await?;
    delete_reaction(&app, &p, &c, None, &r, &h).await
}
async fn list_comment_reactions(
    State(app): Shared,
    Path((a, n, comment)): Path<(String, String, String)>,
    h: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &h, &a).await?;
    let c = card(&app, &p, &n).await?;
    if h.contains_key("x-rizzy-json-request") {
        Ok(Json(reactions_for(&app, &p, &c, Some(&comment), &base(&h)).await?).into_response())
    } else {
        let frame = reaction_page::reactions_html(&app, &p, &c, Some(&comment)).await?;
        Ok(signed_in_frame_page(p.slug, "Reactions", frame))
    }
}
async fn create_comment_reaction(
    State(app): Shared,
    Path((a, n, comment)): Path<(String, String, String)>,
    h: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let p = principal(&app, &h, &a).await?;
    let c = card(&app, &p, &n).await?;
    create_reaction(
        &app,
        &p,
        &c,
        Some(&comment),
        &base(&h),
        &h,
        reaction_body(&h, &body)?,
    )
    .await
}
async fn delete_comment_reaction(
    State(app): Shared,
    Path((a, n, comment, reaction)): Path<(String, String, String, String)>,
    h: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &h, &a).await?;
    let c = card(&app, &p, &n).await?;
    delete_reaction(&app, &p, &c, Some(&comment), &reaction, &h).await
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
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, destination)],
        Html(String::new()),
    )
        .into_response())
}

async fn home() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../static/index.html"))
}
