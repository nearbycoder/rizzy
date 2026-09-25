use axum::{http::HeaderMap, response::Html};
use sqlx::{Row, sqlite::SqliteRow};
use std::collections::HashMap;

use super::{App, Principal, Result, card_page::render_template, db_err, escape, events_page};

const TEMPLATE: &str = include_str!("../static/user.html");

fn page(title: &str, account: i64, content: String) -> Html<String> {
    let values = HashMap::from([
        ("TITLE", escape(title)),
        ("ACCOUNT", account.to_string()),
        ("CONTENT", content),
        (
            "EVENTS_CSS",
            include_str!("../static/events.css").to_owned(),
        ),
        ("EVENTS_JS", include_str!("../static/events.js").to_owned()),
    ]);
    Html(render_template(TEMPLATE, &values))
}

pub(super) async fn show(
    app: &App,
    principal: &Principal,
    user: &SqliteRow,
    headers: &HeaderMap,
) -> Result<Html<String>> {
    let id: String = user.get("id");
    let name: String = user.get("name");
    let email: String = sqlx::query_scalar("SELECT email_address FROM identities WHERE id=?")
        .bind(user.get::<String, _>("identity_id"))
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let self_profile = principal.user_id == id;
    let path = format!("/{}/users/{}", principal.slug, escape(&id));
    let edit = if self_profile {
        format!("<a class=\"button\" href=\"{path}/edit\">Edit profile</a>")
    } else {
        String::new()
    };
    let email_html = if user.get::<Option<String>, _>("verified_at").is_some() {
        format!(
            "<a href=\"mailto:{}\">{}</a>",
            escape(&email),
            escape(&email)
        )
    } else {
        "<p>Unverified</p><p>A sign-in code has been sent to this email address, but the user has not yet logged in to confirm their identity.</p>".to_owned()
    };
    let sign_out = if self_profile {
        "<button class=\"button button--secondary\" type=\"button\" id=\"sign-out\">Sign out of Rizzy on this device</button>"
    } else {
        ""
    };
    let timeline = if user.get::<Option<String>, _>("verified_at").is_some() {
        events_page::user_fragment(app, principal, user, headers, None).await?
    } else {
        String::new()
    };
    let content = format!(
        "<div class=\"profile-card\"><div class=\"profile-card__actions\">{edit}</div><img class=\"profile-avatar\" src=\"{path}/avatar\" alt=\"Profile avatar for {}\"><h1>{}</h1><div class=\"profile-email\">{email_html}</div>{sign_out}</div>{timeline}",
        escape(&name),
        escape(&name)
    );
    Ok(page(&name, principal.slug, content))
}

pub(super) fn edit(
    principal: &Principal,
    user: &SqliteRow,
    email: &str,
    error: Option<&str>,
) -> Html<String> {
    let id: String = user.get("id");
    let name: String = user.get("name");
    let path = format!("/{}/users/{}", principal.slug, escape(&id));
    let error = error
        .map(|message| format!("<p class=\"form-error\">{}</p>", escape(message)))
        .unwrap_or_default();
    let content = format!(
        "<a href=\"{path}\">← Back to profile</a><div class=\"profile-card\"><h1>Edit your profile</h1>{error}<img class=\"profile-avatar\" src=\"{path}/avatar\" alt=\"Profile avatar for {}\"><form class=\"profile-form\" action=\"{path}\" method=\"post\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><label>Name<input type=\"text\" name=\"user[name]\" value=\"{}\" autocomplete=\"name\" required autofocus></label><label>Email address<input type=\"email\" value=\"{}\" readonly></label><div class=\"profile-form__actions\"><button class=\"button\" type=\"submit\">Save changes</button><a href=\"{path}\">Cancel and go back</a></div></form></div>",
        escape(&name),
        escape(&name),
        escape(email)
    );
    page("Edit your profile", principal.slug, content)
}
