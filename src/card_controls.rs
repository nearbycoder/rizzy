use super::*;

#[derive(Clone, Copy)]
enum Control {
    Pin,
    Watch,
}

impl Control {
    fn name(self) -> &'static str {
        match self {
            Self::Pin => "pin",
            Self::Watch => "watch",
        }
    }
}

async fn enabled(
    app: &App,
    principal: &Principal,
    card_id: &str,
    control: Control,
) -> Result<bool> {
    let active: i64 = match control {
        Control::Pin => {
            sqlx::query_scalar("SELECT COUNT(*) FROM pins WHERE card_id=? AND user_id=?")
                .bind(card_id)
                .bind(&principal.user_id)
                .fetch_one(&app.db)
                .await
                .map_err(db_err)?
        }
        Control::Watch => sqlx::query_scalar(
            "SELECT COALESCE((SELECT watching FROM watches WHERE card_id=? AND user_id=?),0)",
        )
        .bind(card_id)
        .bind(&principal.user_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?,
    };
    Ok(active != 0)
}

fn button(account: i64, number: i64, card_id: &str, control: Control, active: bool) -> String {
    let (label, method, icon) = match (control, active) {
        (Control::Pin, true) => ("Unpin this card", "delete", "◆"),
        (Control::Pin, false) => ("Pin this card", "", "◇"),
        (Control::Watch, true) => ("Stop watching", "delete", "◉"),
        (Control::Watch, false) => ("Watch this", "", "◎"),
    };
    let path = format!("/{account}/cards/{number}/{}", control.name());
    let id = escape(card_id);
    let method_input = if method.is_empty() {
        String::new()
    } else {
        format!("<input type=\"hidden\" name=\"_method\" value=\"{method}\">")
    };
    let reversed = if active { " btn--reversed" } else { "" };
    format!(
        "<div id=\"{}_button_card_{id}\"><form method=\"post\" action=\"{path}\">{method_input}<button class=\"btn btn--circle-mobile{reversed}\" type=\"submit\" aria-label=\"{label}\" title=\"{label}\">{icon}<span class=\"for-screen-reader\">{label}</span></button></form></div>",
        control.name()
    )
}

async fn show(
    app: &App,
    account: &str,
    number: &str,
    headers: &HeaderMap,
    control: Control,
) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let principal = principal(app, headers, account).await?;
    let card = card(app, &principal, number).await?;
    let card_id: String = card.get("id");
    let active = enabled(app, &principal, &card_id, control).await?;
    let etag = format!(
        "W/\"{}\"",
        hex::encode(Sha256::digest(
            format!(
                "{}:{}:{}:{}",
                principal.user_id,
                card_id,
                control.name(),
                active
            )
            .as_bytes()
        ))
    );
    let not_modified = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|part| part.trim() == etag));
    let mut response = if not_modified {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Html(format!(
            "<turbo-frame id=\"{}_card_{}\">{}</turbo-frame>",
            control.name(),
            escape(&card_id),
            button(
                principal.slug,
                card.get("number"),
                &card_id,
                control,
                active
            )
        ))
        .into_response()
    };
    response
        .headers_mut()
        .insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=0, must-revalidate"),
    );
    Ok(response)
}

pub(super) async fn show_pin(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    show(&app, &account, &number, &headers, Control::Pin).await
}
pub(super) async fn show_watch(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    show(&app, &account, &number, &headers, Control::Watch).await
}

pub(super) async fn watcher_section(
    app: &App,
    principal: &Principal,
    card_id: &str,
) -> Result<String> {
    let statement = if app.mysql {
        "SELECT u.id,u.name FROM watches w JOIN users u ON u.id=w.user_id WHERE w.card_id=? AND w.watching=1 AND u.active=1 ORDER BY LOWER(u.name),u.id"
    } else {
        "SELECT u.id,u.name FROM watches w JOIN users u ON u.id=w.user_id WHERE w.card_id=? AND w.watching=1 AND u.active=1 ORDER BY u.name COLLATE NOCASE,u.id"
    };
    let watchers = sqlx::query(statement)
        .bind(card_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let count = watchers.len();
    let avatars=watchers.iter().map(|row|{
        let id:String=row.get("id");
        let name:String=row.get("name");
        format!("<a href=\"/{}/users/{}\"><img src=\"/{}/users/{}/avatar\" alt=\"{}\" width=\"30\" height=\"30\"></a>",principal.slug,escape(&id),principal.slug,escape(&id),escape(&name))
    }).collect::<String>();
    Ok(format!(
        "<div id=\"comment_watchers_card_{}\" class=\"comments__subscribers\"><strong>Subscribers</strong><p>{count} {} will be notified when someone comments on this.</p><div>{avatars}</div></div>",
        escape(card_id),
        if count == 1 { "person" } else { "people" }
    ))
}

async fn mutate(
    app: &App,
    account: &str,
    number: &str,
    headers: &HeaderMap,
    body: &Bytes,
    control: Control,
    enabled_state: bool,
) -> Result<Response> {
    let mut state = enabled_state;
    if enabled_state
        && headers
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"))
    {
        if url::form_urlencoded::parse(body)
            .any(|(key, value)| key == "_method" && value == "delete")
        {
            state = false;
        }
    }
    let principal = principal(app, headers, account).await?;
    match control {
        Control::Pin => {
            set_pin(app, &principal, number, state).await?;
        }
        Control::Watch => {
            set_watch(app, &principal, number, state).await?;
        }
    }
    if headers.contains_key("x-rizzy-json-request") {
        return Ok(StatusCode::NO_CONTENT.into_response());
    }
    let card = card(app, &principal, number).await?;
    let card_id: String = card.get("id");
    let accept = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("");
    if accept.contains("text/vnd.turbo-stream.html") {
        let mut stream = format!(
            "<turbo-stream action=\"replace\" target=\"{}_button_card_{}\"><template>{}</template></turbo-stream>",
            control.name(),
            escape(&card_id),
            button(principal.slug, card.get("number"), &card_id, control, state)
        );
        if matches!(control, Control::Watch) {
            stream.push_str(&format!("<turbo-stream action=\"replace\" target=\"comment_watchers_card_{}\"><template>{}</template></turbo-stream>",escape(&card_id),watcher_section(app,&principal,&card_id).await?));
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
    Ok((
        StatusCode::FOUND,
        [(header::LOCATION, format!("/{account}/cards/{number}"))],
    )
        .into_response())
}

pub(super) async fn pin(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    mutate(&app, &account, &number, &headers, &body, Control::Pin, true).await
}
pub(super) async fn unpin(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    mutate(
        &app,
        &account,
        &number,
        &headers,
        &Bytes::new(),
        Control::Pin,
        false,
    )
    .await
}
pub(super) async fn watch(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    mutate(
        &app,
        &account,
        &number,
        &headers,
        &body,
        Control::Watch,
        true,
    )
    .await
}
pub(super) async fn unwatch(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    mutate(
        &app,
        &account,
        &number,
        &headers,
        &Bytes::new(),
        Control::Watch,
        false,
    )
    .await
}
