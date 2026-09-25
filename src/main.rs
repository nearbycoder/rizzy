use axum::{
    Json, Router,
    extract::{OriginalUri, Path, Query, RawQuery, State},
    http::{HeaderMap, HeaderValue, StatusCode, Uri, header},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
};
use chrono::Utc;
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
    sync::Arc,
    time::Duration,
};

struct App {
    db: SqlitePool,
    write_lock: tokio::sync::Mutex<()>,
}
type Shared = State<Arc<App>>;
type Result<T> = std::result::Result<T, ApiError>;

#[derive(Debug)]
struct ApiError(StatusCode, String);
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(json!({"message": self.1}))).into_response()
    }
}
fn err(status: StatusCode, message: &str) -> ApiError {
    ApiError(status, message.into())
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
fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn rich_html(s: &str) -> String {
    format!(
        "<div class=\"action-text-content\">\n  {}\n</div>\n",
        escape(s)
    )
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
        let link = format!(
            "<{}{path}?{query}>; rel=\"next\"",
            base(headers),
            path = uri.path()
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
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Bearer token required"))?;
    let r = sqlx::query("SELECT a.id account_id,u.id user_id,u.role,t.permission FROM access_tokens t JOIN users u ON u.identity_id=t.identity_id JOIN accounts a ON a.id=u.account_id WHERE t.token_hash=? AND a.external_account_id=? AND u.active=1")
        .bind(hash(token)).bind(slug).fetch_optional(&app.db).await.map_err(db_err)?
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED,"Invalid token or account"))?;
    Ok(Principal {
        account_id: r.get("account_id"),
        user_id: r.get("user_id"),
        role: r.get("role"),
        permission: r.get("permission"),
        slug,
    })
}
async fn board(app: &App, p: &Principal, id: &str) -> Result<SqliteRow> {
    sqlx::query("SELECT b.* FROM boards b WHERE b.id=? AND b.account_id=? AND (b.all_access=1 OR EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?))")
        .bind(id.trim_end_matches(".json")).bind(&p.account_id).bind(&p.user_id).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)
}
async fn card(app: &App, p: &Principal, number: &str) -> Result<SqliteRow> {
    let n: i64 = number
        .trim_end_matches(".json")
        .parse()
        .map_err(|_| missing())?;
    sqlx::query("SELECT c.* FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.account_id=? AND c.number=? AND (b.all_access=1 OR EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?))")
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
    sqlx::query("INSERT INTO comments(id,card_id,creator_id,body,created_at,updated_at) VALUES(?,?,?,?,?,?)")
        .bind(id()).bind(card_id).bind(system).bind(body).bind(&timestamp).bind(&timestamp).execute(&app.db).await.map_err(db_err)?;
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
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,particulars,created_at) VALUES(?,?,?,?,?,?,?,?,?,?)")
        .bind(id()).bind(&p.account_id).bind(board_id).bind(card_id).bind(&p.user_id).bind(eventable_type)
        .bind(eventable_id).bind(action).bind(particulars.to_string()).bind(&timestamp)
        .execute(&mut *tx).await.map_err(db_err)?;
    sqlx::query("UPDATE cards SET last_active_at=? WHERE id=?")
        .bind(&timestamp)
        .bind(card_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(())
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
        value["public_description"] = json!(description);
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
    let number: i64 = r.get("number");
    let tags: Vec<String> = sqlx::query_scalar("SELECT t.title FROM tags t JOIN taggings g ON g.tag_id=t.id WHERE g.card_id=? ORDER BY t.title").bind(&id).fetch_all(&app.db).await.map_err(db_err)?;
    let b = board(app, p, &board_id).await?;
    let mut out = json!({"id":id,"number":number,"title":r.get::<String,_>("title"),"status":r.get::<String,_>("status"),"description":description,"description_html":rich_html(&description),"image_url":null,"has_attachments":false,"tags":tags,"closed":r.get::<i64,_>("closed")!=0,"postponed":r.get::<i64,_>("postponed")!=0,"golden":r.get::<i64,_>("golden")!=0,"last_active_at":r.get::<String,_>("last_active_at"),"created_at":r.get::<String,_>("created_at"),"url":format!("{host}/{}/cards/{number}",p.slug),"board":board_json(app,p,&b,host).await?,"creator":user_json(app,p,&creator,host).await?,"assignees":[],"has_more_assignees":false,"comments_url":format!("{host}/{}/cards/{number}/comments",p.slug),"reactions_url":format!("{host}/{}/cards/{number}/reactions",p.slug)});
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
    if env::args().nth(1).as_deref() == Some("seed") {
        seed(&db).await?;
        return Ok(());
    }
    let app = Arc::new(App {
        db,
        write_lock: tokio::sync::Mutex::new(()),
    });
    let routes = Router::new()
        .route("/up", get(|| async { "ok" }))
        .route("/", get(home))
        .route("/my/identity", get(my_identity))
        .route("/my/access_tokens", get(list_tokens).post(create_token))
        .route(
            "/my/access_tokens/{id}",
            axum::routing::delete(delete_token),
        )
        .route("/{account}/my/identity", get(my_identity))
        .route(
            "/{account}/my/access_tokens",
            get(list_tokens).post(create_token),
        )
        .route(
            "/{account}/my/access_tokens/{id}",
            axum::routing::delete(delete_token),
        )
        .route("/{account}/users", get(list_users))
        .route(
            "/{account}/account/settings",
            get(show_account).put(update_account).patch(update_account),
        )
        .route(
            "/{account}/account/entropy",
            axum::routing::put(update_account_entropy).patch(update_account_entropy),
        )
        .route(
            "/{account}/users/{id}",
            get(show_user)
                .put(update_user)
                .patch(update_user)
                .delete(delete_user),
        )
        .route("/{account}/boards", get(list_boards).post(create_board))
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
            "/{account}/public/boards/{key}/cards/{number}",
            get(public_card),
        )
        .route(
            "/{account}/boards/{id}",
            get(show_board)
                .put(update_board)
                .patch(update_board)
                .delete(delete_board),
        )
        .route(
            "/{account}/boards/{board}/columns",
            get(list_columns).post(create_column),
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
        .route("/{account}/boards/{board}/cards", post(create_card))
        .route("/{account}/cards", get(list_cards))
        .route("/{account}/tags", get(list_tags))
        .route("/{account}/search", get(search_cards))
        .route("/{account}/activities", get(list_activities))
        .route(
            "/{account}/cards/{number}",
            get(show_card)
                .put(update_card)
                .patch(update_card)
                .delete(delete_card),
        )
        .route(
            "/{account}/cards/{number}/closure",
            post(close_card).delete(open_card),
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
            axum::routing::put(move_card).patch(move_card),
        )
        .route("/{account}/cards/{number}/taggings", post(toggle_tag))
        .route(
            "/{account}/cards/{number}/assignments",
            post(toggle_assignment),
        )
        .route(
            "/{account}/cards/{number}/watch",
            post(watch_card).delete(unwatch_card),
        )
        .route(
            "/{account}/cards/{number}/pin",
            post(pin_card).delete(unpin_card),
        )
        .route("/{account}/my/pins", get(list_pins))
        .route(
            "/{account}/cards/{number}/reactions",
            get(list_card_reactions).post(create_card_reaction),
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
            axum::routing::put(update_step)
                .patch(update_step)
                .delete(delete_step),
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
                .delete(delete_comment),
        )
        .route(
            "/{account}/cards/{number}/comments/{comment}/reactions",
            get(list_comment_reactions).post(create_comment_reaction),
        )
        .route(
            "/{account}/cards/{number}/comments/{comment}/reactions/{reaction}",
            axum::routing::delete(delete_comment_reaction),
        )
        .with_state(app);
    let bind = env::var("RIZZY_BIND").unwrap_or_else(|_| "127.0.0.1:3007".into());
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    println!("Rizzy listening on http://{bind}");
    axum::serve(listener, routes).await?;
    Ok(())
}

async fn seed(db: &SqlitePool) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let account_id = id();
    let identity_id = id();
    let user_id = id();
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
    sqlx::query("INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)")
        .bind(&identity_id)
        .bind(email)
        .bind(&timestamp)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES(?,?,?,?,'owner',?)").bind(&user_id).bind(&account_id).bind(&identity_id).bind(name).bind(&timestamp).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES(?,?,?,?,?,?)").bind(&token_id).bind(&identity_id).bind(hash(&token)).bind("Bootstrap token").bind("write").bind(&timestamp).execute(&mut *tx).await?;
    tx.commit().await?;
    println!("Account: {slug}\nBearer token: {token}");
    Ok(())
}

async fn identity_auth(app: &App, headers: &HeaderMap) -> Result<(String, String)> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Bearer token required"))?;
    let r = sqlx::query("SELECT identity_id,permission FROM access_tokens WHERE token_hash=?")
        .bind(hash(token))
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Invalid token"))?;
    Ok((r.get("identity_id"), r.get("permission")))
}
async fn check_my_scope(app: &App, identity_id: &str, uri: &Uri) -> Result<()> {
    let mut parts = uri.path().trim_start_matches('/').split('/');
    if let Ok(slug) = parts.next().unwrap_or("").parse::<i64>() {
        if parts.next() == Some("my") {
            let count:i64=sqlx::query_scalar("SELECT COUNT(*) FROM users u JOIN accounts a ON a.id=u.account_id WHERE u.identity_id=? AND a.external_account_id=? AND u.active=1")
                .bind(identity_id).bind(slug).fetch_one(&app.db).await.map_err(db_err)?;
            if count == 0 {
                return Err(err(StatusCode::FORBIDDEN, "Account access required"));
            }
        }
    }
    Ok(())
}
async fn my_identity(
    State(app): Shared,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Value>> {
    let (identity_id, _) = identity_auth(&app, &headers).await?;
    check_my_scope(&app, &identity_id, &uri).await?;
    let rows=sqlx::query("SELECT a.id account_id,a.external_account_id,a.name,a.created_at,u.id user_id,u.role FROM accounts a JOIN users u ON u.account_id=a.id WHERE u.identity_id=? AND u.active=1 AND u.role!='system' ORDER BY a.created_at")
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
) -> Result<Json<Value>> {
    let (identity_id, _) = identity_auth(&app, &headers).await?;
    check_my_scope(&app, &identity_id, &uri).await?;
    let rows=sqlx::query("SELECT id,description,permission,created_at FROM access_tokens WHERE identity_id=? ORDER BY created_at DESC,id DESC")
        .bind(identity_id).fetch_all(&app.db).await.map_err(db_err)?;
    Ok(Json(json!(rows.iter().map(|r|json!({"id":r.get::<String,_>("id"),"description":r.get::<String,_>("description"),"permission":r.get::<String,_>("permission"),"created_at":r.get::<String,_>("created_at")})).collect::<Vec<_>>())))
}
async fn create_token(
    State(app): Shared,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    Json(body): Json<Value>,
) -> Result<Response> {
    let (identity_id, permission) = identity_auth(&app, &headers).await?;
    check_my_scope(&app, &identity_id, &uri).await?;
    if permission != "write" {
        return Err(err(StatusCode::FORBIDDEN, "Write permission required"));
    }
    let v = nested(&body, "access_token");
    let description = string(v, "description").unwrap_or_default();
    let token_permission = string(v, "permission").unwrap_or_else(|| "read".into());
    if token_permission != "read" && token_permission != "write" {
        return Err(bad("invalid permission"));
    }
    let id = id();
    let token = hex::encode(rand::random::<[u8; 32]>());
    let created = now();
    sqlx::query("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES(?,?,?,?,?,?)")
        .bind(&id).bind(identity_id).bind(hash(&token)).bind(&description).bind(&token_permission).bind(&created).execute(&app.db).await.map_err(db_err)?;
    Ok((StatusCode::CREATED,Json(json!({"id":id,"token":token,"description":description,"permission":token_permission,"created_at":created}))).into_response())
}
async fn delete_token(
    State(app): Shared,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<StatusCode> {
    let (identity_id, permission) = identity_auth(&app, &headers).await?;
    check_my_scope(&app, &identity_id, &uri).await?;
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
    Ok(StatusCode::NO_CONTENT)
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
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    let u = active_user(&app, &p, &id).await?;
    Ok(Json(
        user_json(&app, &p, &u.get::<String, _>("id"), &base(&headers)).await?,
    ))
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
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let u = active_user(&app, &p, &id).await?;
    can_change_user(&p, &u)?;
    let name = string(nested(&body, "user"), "name").unwrap_or_else(|| u.get("name"));
    if name.trim().is_empty() {
        return Err(bad("name is required"));
    }
    sqlx::query("UPDATE users SET name=? WHERE id=?")
        .bind(name)
        .bind(&id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(Json(user_json(&app, &p, &id, &base(&headers)).await?))
}
async fn delete_user(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
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
    sqlx::query("UPDATE users SET active=0 WHERE id=?")
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
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
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    Ok(Json(account_json(&app, &p).await?))
}
async fn update_account(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    p.account_admin()?;
    let name = required(nested(&body, "account"), "name")?;
    sqlx::query("UPDATE accounts SET name=? WHERE id=?")
        .bind(name)
        .bind(&p.account_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
}
async fn update_account_entropy(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    p.account_admin()?;
    let days = nested(&body, "entropy")
        .get("auto_postpone_period_in_days")
        .and_then(Value::as_i64)
        .ok_or_else(|| bad("auto_postpone_period_in_days is required"))?;
    let days = valid_entropy(days)?;
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
    Ok(Json(account_json(&app, &p).await?))
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
    let scope = " FROM boards b WHERE b.account_id=? AND (b.all_access=1 OR EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?))";
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*){scope}"))
        .bind(&p.account_id)
        .bind(&p.user_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let rows = sqlx::query(&format!(
        "SELECT b.*{scope} ORDER BY b.updated_at DESC LIMIT ? OFFSET ?"
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
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    let r = board(&app, &p, &id).await?;
    Ok(Json(board_detail(&app, &p, &r, &base(&headers)).await?))
}
async fn create_board(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response> {
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
    if !all {
        sqlx::query("INSERT INTO accesses(board_id,user_id) VALUES(?,?)")
            .bind(&id)
            .bind(&p.user_id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
    }
    tx.commit().await.map_err(db_err)?;
    let r = board(&app, &p, &id).await?;
    Ok(created(
        format!("/{account}/boards/{id}.json"),
        board_detail(&app, &p, &r, &base(&headers)).await?,
    ))
}
async fn update_board(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response> {
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
    if let Some(ids) = body
        .get("user_ids")
        .or_else(|| v.get("user_ids"))
        .and_then(Value::as_array)
    {
        sqlx::query("DELETE FROM accesses WHERE board_id=?")
            .bind(&id)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        for user in ids.iter().filter_map(Value::as_str) {
            sqlx::query("INSERT INTO accesses(board_id,user_id) SELECT ?,id FROM users WHERE id=? AND account_id=?")
                .bind(&id).bind(user).bind(&p.account_id).execute(&mut *tx).await.map_err(db_err)?;
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
            return Ok(StatusCode::NO_CONTENT.into_response());
        }
    }
    let r = board(&app, &p, &id).await?;
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
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let b = board(&app, &p, &id).await?;
    p.admin(&b.get::<String, _>("creator_id"))?;
    sqlx::query("DELETE FROM boards WHERE id=? AND account_id=?")
        .bind(id)
        .bind(&p.account_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(StatusCode::NO_CONTENT)
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
    let mut body = format!(
        "<h1>{}</h1><p>{}</p><ul>",
        escape(&board.get::<String, _>("name")),
        escape(&board.get::<String, _>("public_description"))
    );
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
    let body = format!(
        "<nav><a href=\"/{account}/public/boards/{key}\">{}</a></nav><h1>{}</h1><article>{}</article>",
        escape(&board.get::<String, _>("name")),
        escape(&title),
        escape(&card.get::<String, _>("description"))
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

#[derive(Default)]
struct CardQuery {
    indexed_by: Option<String>,
    sorted_by: Option<String>,
    board_ids: Vec<String>,
    column_ids: Vec<String>,
    tag_ids: Vec<String>,
    assignee_ids: Vec<String>,
    creator_ids: Vec<String>,
    card_ids: Vec<String>,
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
                "indexed_by" => q.indexed_by = Some(value.into_owned()),
                "sorted_by" => q.sorted_by = Some(value.into_owned()),
                "board_ids[]" => q.board_ids.push(value.into_owned()),
                "column_ids[]" => q.column_ids.push(value.into_owned()),
                "tag_ids[]" => q.tag_ids.push(value.into_owned()),
                "assignee_ids[]" => q.assignee_ids.push(value.into_owned()),
                "creator_ids[]" => q.creator_ids.push(value.into_owned()),
                "card_ids[]" => q.card_ids.push(value.into_owned()),
                "assignment_status" => q.assignment_status = Some(value.into_owned()),
                "page" => q.page = value.parse::<i64>().unwrap_or(1).max(1),
                _ => {}
            }
        }
    }
    q
}
fn cards_query<'a>(select: &str, p: &'a Principal, q: &'a CardQuery) -> QueryBuilder<'a, Sqlite> {
    let mut builder = QueryBuilder::new(format!(
        "{select} FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.account_id="
    ));
    builder.push_bind(&p.account_id).push(" AND c.status='published' AND (b.all_access=1 OR EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=").push_bind(&p.user_id).push("))");
    if q.indexed_by.as_deref() != Some("closed") && q.card_ids.is_empty() {
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
    let pattern = format!(
        "%{}%",
        term.replace('\\', "\\\\")
            .replace('%', "\\%")
            .replace('_', "\\_")
    );
    let filter = " FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.account_id=? AND c.status='published' AND (b.all_access=1 OR EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?)) AND (c.number=? OR (c.title LIKE ? ESCAPE '\\' OR c.description LIKE ? ESCAPE '\\' OR EXISTS (SELECT 1 FROM comments m WHERE m.card_id=c.id AND m.body LIKE ? ESCAPE '\\')))";
    let total: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*){filter}"))
        .bind(&p.account_id)
        .bind(&p.user_id)
        .bind(number)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let rows = sqlx::query(&format!(
        "SELECT c.*{filter} ORDER BY c.last_active_at DESC,c.id DESC LIMIT ? OFFSET ?"
    ))
    .bind(&p.account_id)
    .bind(&p.user_id)
    .bind(number)
    .bind(&pattern)
    .bind(&pattern)
    .bind(&pattern)
    .bind(limit)
    .bind(offset)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let host = base(&headers);
    let mut items = Vec::new();
    for row in &rows {
        items.push(card_json(&app, &p, row, &host, false).await?);
    }
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
    builder.push_bind(&p.account_id).push(" AND (b.all_access=1 OR EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=")
        .push_bind(&p.user_id).push(")) AND (e.eventable_type='Card' OR EXISTS (SELECT 1 FROM comments m WHERE m.id=e.eventable_id AND m.card_id=c.id))");
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
        let assigned = assignments.get(&id).cloned().unwrap_or_default();
        let assignees: Vec<Value> = assigned
            .iter()
            .take(5)
            .filter_map(|id| users.get(id).cloned())
            .collect();
        let mut value = json!({"id":id,"number":number,"title":title,"status":c.get::<String,_>("status"),
            "description":description,"description_html":rich_html(&description),"image_url":null,
            "has_attachments":false,"tags":tags.get(&id).cloned().unwrap_or_default(),
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
            "body":{"plain_text":body,"html":rich_html(&body)},"creator":creator,
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
async fn list_cards(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let query = parse_card_query(raw);
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
    let mut builder = cards_query("SELECT c.*", &p, &query);
    builder
        .push(format!(" ORDER BY {order},c.id DESC LIMIT "))
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
    let rows = builder.build().fetch_all(&app.db).await.map_err(db_err)?;
    let host = base(&headers);
    let mut out = Vec::new();
    for r in &rows {
        out.push(card_json(&app, &p, r, &host, false).await?);
    }
    Ok(paginated(json!(out), total, query.page, &uri, &headers))
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
        "SELECT *{scope} ORDER BY last_active_at DESC,id DESC LIMIT ? OFFSET ?"
    ))
    .bind(&board_id)
    .bind(&column_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let host = base(&headers);
    let mut out = Vec::new();
    for r in &rows {
        out.push(card_json(&app, &p, r, &host, false).await?);
    }
    Ok(paginated(json!(out), total, page, &uri, &headers))
}
async fn show_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    let c = card(&app, &p, &number).await?;
    Ok(Json(card_json(&app, &p, &c, &base(&headers), true).await?))
}
async fn create_card(
    State(app): Shared,
    Path((account, board_id)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    board(&app, &p, &board_id).await?;
    let v = nested(&body, "card");
    let title = string(v, "title")
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "Untitled".into());
    let description = string(v, "description").unwrap_or_default();
    let event_id = id();
    let id = id();
    let timestamp = now();
    let writer = app.write_lock.lock().await;
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let number: i64 = sqlx::query_scalar(
        "UPDATE accounts SET cards_count=cards_count+1 WHERE id=? RETURNING cards_count",
    )
    .bind(&p.account_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_err)?;
    sqlx::query("INSERT INTO cards(id,account_id,board_id,creator_id,number,title,description,created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,?,?,?,?,?)")
        .bind(&id).bind(&p.account_id).bind(&board_id).bind(&p.user_id).bind(number).bind(title).bind(description).bind(&timestamp).bind(&timestamp).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
    sqlx::query("INSERT INTO watches(card_id,user_id,watching) VALUES(?,?,1)")
        .bind(&id)
        .bind(&p.user_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,particulars,created_at) VALUES(?,?,?,?,?,'Card',?,'card_published','{}',?)")
        .bind(event_id).bind(&p.account_id).bind(&board_id).bind(&id).bind(&p.user_id)
        .bind(&id).bind(&timestamp).execute(&mut *tx).await.map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    drop(writer);
    let c = card(&app, &p, &number.to_string()).await?;
    Ok(created(
        format!("/{account}/cards/{number}.json"),
        card_json(&app, &p, &c, &base(&headers), true).await?,
    ))
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
    let title = string(v, "title").unwrap_or_else(|| c.get("title"));
    let old_title: String = c.get("title");
    let old_status: String = c.get("status");
    let description = string(v, "description").unwrap_or_else(|| c.get("description"));
    let status = string(v, "status").unwrap_or_else(|| c.get("status"));
    if !["drafted", "published"].contains(&status.as_str()) {
        return Err(bad("invalid status"));
    }
    sqlx::query(
        "UPDATE cards SET title=?,description=?,status=?,updated_at=?,last_active_at=? WHERE id=?",
    )
    .bind(&title)
    .bind(description)
    .bind(&status)
    .bind(now())
    .bind(now())
    .bind(c.get::<String, _>("id"))
    .execute(&app.db)
    .await
    .map_err(db_err)?;
    if title != old_title && !old_title.is_empty() {
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
    if old_status == "drafted" && status == "published" {
        record_event(
            &app,
            &p,
            &c.get::<String, _>("board_id"),
            &c.get::<String, _>("id"),
            "Card",
            &c.get::<String, _>("id"),
            "card_published",
            json!({}),
        )
        .await?;
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
    sqlx::query("DELETE FROM cards WHERE id=?")
        .bind(c.get::<String, _>("id"))
        .execute(&app.db)
        .await
        .map_err(db_err)?;
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
    sqlx::query(sql)
        .bind(value)
        .bind(&timestamp)
        .bind(&timestamp)
        .bind(c.get::<String, _>("id"))
        .execute(&app.db)
        .await
        .map_err(db_err)?;
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
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let column = required(&body, "column_id")?;
    let col = checked_column(&app, &p, &c.get::<String, _>("board_id"), &column).await?;
    let timestamp = now();
    sqlx::query("UPDATE cards SET column_id=?,closed=0,postponed=0,updated_at=?,last_active_at=? WHERE id=?")
        .bind(column).bind(&timestamp).bind(&timestamp).bind(c.get::<String,_>("id")).execute(&app.db).await.map_err(db_err)?;
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
    Ok(StatusCode::NO_CONTENT)
}
async fn untriage_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let timestamp = now();
    sqlx::query("UPDATE cards SET column_id=NULL,closed=0,postponed=0,updated_at=?,last_active_at=? WHERE id=?")
        .bind(&timestamp).bind(&timestamp).bind(c.get::<String,_>("id")).execute(&app.db).await.map_err(db_err)?;
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
    Ok(StatusCode::NO_CONTENT)
}
async fn move_card(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let destination = required(&body, "board_id")?;
    let old_board = board(&app, &p, &c.get::<String, _>("board_id")).await?;
    let new_board = board(&app, &p, &destination).await?;
    sqlx::query("UPDATE cards SET board_id=?,column_id=NULL,updated_at=? WHERE id=?")
        .bind(&destination)
        .bind(now())
        .bind(c.get::<String, _>("id"))
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    if c.get::<String, _>("board_id") != destination {
        sqlx::query("UPDATE events SET board_id=? WHERE card_id=?")
            .bind(&destination)
            .bind(c.get::<String, _>("id"))
            .execute(&app.db)
            .await
            .map_err(db_err)?;
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
    Ok(Json(card_json(&app, &p, &c, &base(&headers), true).await?))
}
async fn toggle_tag(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let title = required(&body, "tag_title")?
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
    Ok(StatusCode::NO_CONTENT)
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
        (body.clone(), rich_html(&body))
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
    Json(body): Json<Value>,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    if c.get::<String, _>("status") != "published" {
        return Err(err(StatusCode::FORBIDDEN, "Card is not commentable"));
    }
    let content = required(nested(&body, "comment"), "body")?;
    let id = id();
    let timestamp = now();
    sqlx::query("INSERT INTO comments(id,card_id,creator_id,body,created_at,updated_at) VALUES(?,?,?,?,?,?)")
        .bind(&id)
        .bind(c.get::<String, _>("id"))
        .bind(&p.user_id)
        .bind(&content)
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
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
    let r = comment_row(&app, &c, &id).await?;
    Ok(created(
        format!("/{account}/cards/{number}/comments/{id}.json"),
        comment_json(&app, &p, &c, &r, &base(&headers)).await?,
    ))
}
async fn show_comment(
    State(app): Shared,
    Path((account, number, id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    let c = card(&app, &p, &number).await?;
    let r = comment_row(&app, &c, &id).await?;
    Ok(Json(comment_json(&app, &p, &c, &r, &base(&headers)).await?))
}
async fn update_comment(
    State(app): Shared,
    Path((account, number, id)): Path<(String, String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let r = comment_row(&app, &c, &id).await?;
    if r.get::<String, _>("creator_id") != p.user_id {
        return Err(err(StatusCode::FORBIDDEN, "Comment creator required"));
    }
    let content = required(nested(&body, "comment"), "body")?;
    sqlx::query("UPDATE comments SET body=?,updated_at=? WHERE id=?")
        .bind(content)
        .bind(now())
        .bind(&id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    let r = comment_row(&app, &c, &id).await?;
    Ok(Json(comment_json(&app, &p, &c, &r, &base(&headers)).await?))
}
async fn delete_comment(
    State(app): Shared,
    Path((account, number, comment)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let r = comment_row(&app, &c, &comment).await?;
    if r.get::<String, _>("creator_id") != p.user_id {
        return Err(err(StatusCode::FORBIDDEN, "Comment creator required"));
    }
    sqlx::query(
        "DELETE FROM events WHERE eventable_type='Comment' AND eventable_id=? AND card_id=?",
    )
    .bind(&comment)
    .bind(c.get::<String, _>("id"))
    .execute(&app.db)
    .await
    .map_err(db_err)?;
    sqlx::query(
        "DELETE FROM reactions WHERE reactable_type='Comment' AND reactable_id=? AND account_id=?",
    )
    .bind(&comment)
    .bind(&p.account_id)
    .execute(&app.db)
    .await
    .map_err(db_err)?;
    let done = sqlx::query("DELETE FROM comments WHERE id=? AND card_id=? AND creator_id=?")
        .bind(comment)
        .bind(c.get::<String, _>("id"))
        .bind(&p.user_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    if done.rows_affected() == 0 {
        return Err(missing());
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn toggle_assignment(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let c = card(&app, &p, &number).await?;
    let assignee_id = required(&body, "assignee_id")?;
    let board_id: String = c.get("board_id");
    let card_id: String = c.get("id");
    let assignee=sqlx::query("SELECT u.id,u.name FROM users u JOIN boards b ON b.id=? WHERE u.id=? AND u.account_id=? AND u.active=1 AND u.role!='system' AND (b.all_access=1 OR EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=u.id))")
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
async fn watch_card(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &h, &a).await?;
    set_watch(&app, &p, &n, true).await
}
async fn unwatch_card(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &h, &a).await?;
    set_watch(&app, &p, &n, false).await
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
async fn pin_card(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &h, &a).await?;
    set_pin(&app, &p, &n, true).await
}
async fn unpin_card(
    State(app): Shared,
    Path((a, n)): Path<(String, String)>,
    h: HeaderMap,
) -> Result<StatusCode> {
    let p = principal(&app, &h, &a).await?;
    set_pin(&app, &p, &n, false).await
}
async fn list_pins(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Json<Value>> {
    let p = principal(&app, &headers, &account).await?;
    let rows=sqlx::query("SELECT c.* FROM pins pin JOIN cards c ON c.id=pin.card_id JOIN boards b ON b.id=c.board_id WHERE pin.user_id=? AND pin.account_id=? AND (b.all_access=1 OR EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=?)) ORDER BY pin.created_at DESC,pin.id DESC LIMIT 100")
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

async fn home() -> axum::response::Html<&'static str> {
    axum::response::Html(include_str!("../static/index.html"))
}
