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
        "<a href=\"{path}\">← Back to profile</a><div class=\"profile-card\"><h1>Edit your profile</h1>{error}<img class=\"profile-avatar\" src=\"{path}/avatar\" alt=\"Profile avatar for {}\"><form class=\"profile-form\" action=\"{path}\" method=\"post\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><label>Name<input type=\"text\" name=\"user[name]\" value=\"{}\" autocomplete=\"name\" required autofocus></label><label>Email address<input type=\"email\" value=\"{}\" readonly></label><a href=\"{path}/email_addresses/new\">Change email</a><div class=\"profile-form__actions\"><button class=\"button\" type=\"submit\">Save changes</button><a href=\"{path}\">Cancel and go back</a></div></form></div>",
        escape(&name),
        escape(&name),
        escape(email)
    );
    page("Edit your profile", principal.slug, content)
}

pub(super) fn new_email(principal: &Principal, user_id: &str) -> Html<String> {
    let path = format!(
        "/{}/users/{}/email_addresses",
        principal.slug,
        escape(user_id)
    );
    let content = format!(
        "<a href=\"/{}/users/{}/edit\">← My profile</a><div class=\"profile-card\"><h1>Change your email</h1><form class=\"profile-form\" method=\"post\" action=\"{path}\"><label>New email address<input type=\"email\" name=\"email_address\" placeholder=\"New email address\" autocomplete=\"email\" required autofocus></label><p>Enter your new email address, then check your email to confirm the change.</p><button class=\"button\" type=\"submit\">Continue</button></form></div>",
        principal.slug,
        escape(user_id)
    );
    page("Change your email", principal.slug, content)
}

pub(super) fn email_sent(principal: &Principal, user_id: &str, email: &str) -> Html<String> {
    let edit = format!("/{}/users/{}/edit", principal.slug, escape(user_id));
    let content = format!(
        "<a href=\"{edit}\">← My profile</a><div class=\"profile-card\"><h1>Check your email</h1><p>We just sent an email to <strong>{}</strong></p><p>Hit the link in the email to confirm this is the email address you want to use with Rizzy going forward.</p><a class=\"button\" href=\"{edit}\">Done</a></div>",
        escape(email)
    );
    page("Confirm your new email address", principal.slug, content)
}

pub(super) fn confirm_email(account: i64, user_id: &str, token: &str) -> Html<String> {
    let action = format!(
        "/{account}/users/{}/email_addresses/{}/confirmation",
        escape(user_id),
        escape(token)
    );
    let content = format!(
        "<div class=\"profile-card\"><h1>Confirm email change</h1><p>Just a sec while we confirm your new email address.</p><form id=\"email-confirmation\" method=\"post\" action=\"{action}\"><input type=\"hidden\" name=\"email_address_token\" value=\"{}\"><button class=\"button\" type=\"submit\">Done</button></form></div><script>document.getElementById('email-confirmation')?.requestSubmit()</script>",
        escape(token)
    );
    page("Confirm email change", account, content)
}

pub(super) fn expired_email(account: i64, user_id: &str) -> Html<String> {
    let change = format!("/{account}/users/{}/email_addresses/new", escape(user_id));
    let content = format!(
        "<div class=\"profile-card\"><h1>Link expired</h1><p>That email confirmation link is no longer valid—they expire after 30 minutes. You’ll have to try again.</p><a class=\"button\" href=\"{change}\">Change my email address</a></div>"
    );
    page("Link expired", account, content)
}
