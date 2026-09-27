use super::*;

pub(super) async fn ensure_token(app: &App, user_id: &str) -> Result<String> {
    let token = random_token();
    let timestamp = now();
    let expiry = (Utc::now() + chrono::Duration::days(30))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let statement = if app.mysql {
        "INSERT INTO notification_unsubscribe_tokens(user_id,token,expires_at,created_at) VALUES(?,?,?,?) ON DUPLICATE KEY UPDATE token=IF(expires_at<=?,VALUES(token),token),created_at=IF(expires_at<=?,VALUES(created_at),created_at),expires_at=IF(expires_at<=?,VALUES(expires_at),expires_at)"
    } else {
        "INSERT INTO notification_unsubscribe_tokens(user_id,token,expires_at,created_at) VALUES(?,?,?,?) ON CONFLICT(user_id) DO UPDATE SET token=excluded.token,expires_at=excluded.expires_at,created_at=excluded.created_at WHERE notification_unsubscribe_tokens.expires_at<=?"
    };
    let mut query = sqlx::query(statement)
        .bind(user_id)
        .bind(&token)
        .bind(expiry)
        .bind(&timestamp)
        .bind(&timestamp);
    if app.mysql {
        query = query.bind(&timestamp).bind(&timestamp);
    }
    query.execute(&app.db).await.map_err(db_err)?;
    sqlx::query_scalar("SELECT token FROM notification_unsubscribe_tokens WHERE user_id=?")
        .bind(user_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)
}

async fn token_user(app: &App, account: &str, token: Option<&str>) -> Result<Option<DbRow>> {
    let Some(token) = token.filter(|token| !token.is_empty()) else {
        return Ok(None);
    };
    sqlx::query("SELECT u.id,u.name,u.account_id FROM notification_unsubscribe_tokens t JOIN users u ON u.id=t.user_id JOIN accounts a ON a.id=u.account_id WHERE t.token=? AND t.expires_at>? AND a.external_account_id=?")
        .bind(token).bind(now()).bind(account.parse::<i64>().unwrap_or(-1))
        .fetch_optional(&app.db).await.map_err(db_err)
}

fn root(account: &str) -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, format!("/{account}/"))],
        Html(String::new()),
    )
        .into_response()
}

pub(super) async fn new(
    State(app): Shared,
    Path(account): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response> {
    let token = query.get("access_token");
    let Some(user) = token_user(&app, &account, token.map(String::as_str)).await? else {
        return Ok(root(&account));
    };
    let name: String = user.get("name");
    let token = token.expect("validated token is present");
    Ok(Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Unsubscribe | Rizzy</title></head><body><main><p>Unsubscribing from all email notifications as {}…</p><form id=\"unsubscribe\" method=\"post\" action=\"/{account}/notifications/unsubscribe?access_token={token}\"><button type=\"submit\">Unsubscribe now</button></form></main><script>document.getElementById('unsubscribe').requestSubmit()</script></body></html>",
        escape(&name)
    )).into_response())
}

pub(super) async fn show(
    State(app): Shared,
    Path(account): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response> {
    let Some(user) = token_user(
        &app,
        &account,
        query.get("access_token").map(String::as_str),
    )
    .await?
    else {
        return Ok(root(&account));
    };
    let name: String = user.get("name");
    let first = name.split_whitespace().next().unwrap_or(&name);
    Ok(Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Unsubscribed | Rizzy</title></head><body><main><h1>You’re unsubscribed</h1><p>Thanks, {}! Rizzy won’t send you any more email notifications. If you change your mind, you can turn them back on in <a href=\"/{account}/notifications/settings\">Notification Settings</a>.</p><a href=\"/{account}/\">Back to Rizzy</a></main></body></html>",
        escape(first)
    )).into_response())
}

pub(super) async fn create(
    State(app): Shared,
    Path(account): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Result<Response> {
    let token = query.get("access_token");
    let Some(user) = token_user(&app, &account, token.map(String::as_str)).await? else {
        return Ok(root(&account));
    };
    let user_id: String = user.get("id");
    let account_id: String = user.get("account_id");
    let statement = if app.mysql {
        "INSERT INTO notification_settings(user_id,account_id,bundle_email_frequency) VALUES(?,?,'never') ON DUPLICATE KEY UPDATE bundle_email_frequency='never'"
    } else {
        "INSERT INTO notification_settings(user_id,account_id,bundle_email_frequency) VALUES(?,?,'never') ON CONFLICT(user_id) DO UPDATE SET bundle_email_frequency='never' WHERE notification_settings.bundle_email_frequency<>'never'"
    };
    sqlx::query(statement)
        .bind(&user_id)
        .bind(&account_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    sqlx::query("DELETE FROM notification_bundles WHERE user_id=? AND status=0")
        .bind(&user_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    let token = token.expect("validated token is present");
    Ok((
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("/{account}/notifications/unsubscribe?access_token={token}"),
        )],
    )
        .into_response())
}
