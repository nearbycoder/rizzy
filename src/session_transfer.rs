use super::*;
use base64::Engine as _;
use qrcodegen::{QrCode, QrCodeEcc};

fn active_cutoff() -> String {
    (Utc::now() - chrono::Duration::hours(4)).to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

pub(super) async fn active_token(app: &App, identity_id: &str) -> Result<Option<String>> {
    sqlx::query_scalar("SELECT token FROM identity_transfers WHERE identity_id=? AND created_at>=? ORDER BY created_at DESC LIMIT 1")
        .bind(identity_id)
        .bind(active_cutoff())
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)
}

pub(super) fn qr_link(app: &App, slug: i64, destination: &str) -> String {
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .encode(json!({"account":slug,"url":destination}).to_string());
    let signature = hmac_sha256(&app.qr_secret, &format!("qr_code:{payload}"));
    format!("{payload}--{signature}")
}

pub(super) async fn create(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let principal = principal(&app, &headers, &account).await?;
    principal.write()?;
    let identity_id: Option<String> =
        sqlx::query_scalar("SELECT identity_id FROM users WHERE id=?")
            .bind(&principal.user_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
    let identity_id = identity_id.ok_or_else(missing)?;
    let token = random_token();
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("DELETE FROM identity_transfers WHERE identity_id=?")
        .bind(&identity_id)
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query("INSERT INTO identity_transfers(id,identity_id,token,created_at) VALUES(?,?,?,?)")
        .bind(id())
        .bind(identity_id)
        .bind(token)
        .bind(now())
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    Ok((
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("/{}/users/{}", principal.slug, principal.user_id),
        )],
    )
        .into_response())
}

pub(super) async fn show(Path(token): Path<String>) -> Html<String> {
    let action = format!("/session/transfers/{}", escape(&token));
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Sign in on this device | Rizzy</title></head><body><main><h1>Signing in on this device</h1><form id=\"transfer-form\" method=\"post\" action=\"{action}\"><button type=\"submit\">Continue</button></form></main><script>document.getElementById('transfer-form').requestSubmit()</script></body></html>"
    ))
}

pub(super) async fn qr_code(
    State(app): Shared,
    Path((account, token)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let account_exists: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM accounts WHERE external_account_id=?")
            .bind(slug)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
    if account_exists == 0 {
        return Ok((
            StatusCode::FOUND,
            [(header::LOCATION, "/session/menu")],
            Html(String::new()),
        )
            .into_response());
    }
    let invalid_link = || {
        if headers.contains_key("x-rizzy-json-request") {
            source_json_internal_error()
        } else {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html(include_str!("../static/500.html")),
            )
                .into_response()
        }
    };
    let Some((payload, signature)) = token.rsplit_once("--") else {
        return Ok(invalid_link());
    };
    let expected = hmac_sha256(&app.qr_secret, &format!("qr_code:{payload}"));
    if signature.len() != expected.len()
        || signature
            .bytes()
            .zip(expected.bytes())
            .fold(0_u8, |difference, (left, right)| {
                difference | (left ^ right)
            })
            != 0
    {
        return Ok(invalid_link());
    }
    let Ok(decoded) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload) else {
        return Ok(invalid_link());
    };
    let Ok(signed): std::result::Result<Value, _> = serde_json::from_slice(&decoded) else {
        return Ok(invalid_link());
    };
    if signed.get("account").and_then(Value::as_i64) != Some(slug) {
        return Ok(invalid_link());
    }
    let Some(url) = signed.get("url").and_then(Value::as_str) else {
        return Ok(invalid_link());
    };
    let qr = QrCode::encode_text(&url, QrCodeEcc::Medium)
        .map_err(|_| err(StatusCode::BAD_REQUEST, "Link too long for QR code"))?;
    let size = qr.size();
    let mut path = String::new();
    for y in 0..size {
        for x in 0..size {
            if qr.get_module(x, y) {
                path.push_str(&format!("M{x} {y}h1v1h-1z"));
            }
        }
    }
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"-16 -16 {} {}\" shape-rendering=\"crispEdges\"><rect x=\"-16\" y=\"-16\" width=\"100%\" height=\"100%\" fill=\"white\"/><path fill=\"black\" d=\"{path}\"/></svg>",
        size + 32,
        size + 32
    );
    Ok((
        [
            (header::CONTENT_TYPE, "image/svg+xml"),
            (header::CACHE_CONTROL, "max-age=31556952, public"),
        ],
        svg,
    )
        .into_response())
}

pub(super) async fn redeem(
    State(app): Shared,
    Path(token): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let mut tx = app.db.begin().await.map_err(db_err)?;
    let cutoff = active_cutoff();
    let identity_id: Option<String> = if app.mysql {
        let found = sqlx::query_scalar(
            "SELECT identity_id FROM identity_transfers WHERE token=? AND created_at>=? FOR UPDATE",
        )
        .bind(&token)
        .bind(&cutoff)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?;
        if found.is_some() {
            let deleted =
                sqlx::query("DELETE FROM identity_transfers WHERE token=? AND created_at>=?")
                    .bind(&token)
                    .bind(&cutoff)
                    .execute(&mut *tx)
                    .await
                    .map_err(db_err)?;
            if deleted.rows_affected() == 0 {
                return Ok(StatusCode::BAD_REQUEST.into_response());
            }
        }
        found
    } else {
        sqlx::query_scalar(
            "DELETE FROM identity_transfers WHERE token=? AND created_at>=? RETURNING identity_id",
        )
        .bind(&token)
        .bind(&cutoff)
        .fetch_optional(&mut *tx)
        .await
        .map_err(db_err)?
    };
    let Some(identity_id) = identity_id else {
        return Ok(StatusCode::BAD_REQUEST.into_response());
    };
    let session = random_token();
    sqlx::query("INSERT INTO sessions(id,identity_id,token_hash,user_agent,signup_pending,created_at) VALUES(?,?,?,?,0,?)")
        .bind(id())
        .bind(identity_id)
        .bind(hash(&session))
        .bind(headers.get(header::USER_AGENT).and_then(|value| value.to_str().ok()))
        .bind(now())
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    let mut response = (StatusCode::FOUND, [(header::LOCATION, "/session/menu")]).into_response();
    set_cookie(
        response.headers_mut(),
        "session_token",
        &session,
        31536000,
        secure_request(&headers),
    );
    Ok(response)
}
