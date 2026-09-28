use axum::{http::HeaderMap, response::Html};
use sqlx::{Row, any::AnyRow as DbRow};
use std::collections::HashMap;

use super::{
    App, Principal, Result, base, card_page::render_template, db_err, escape, events_page,
    session_transfer,
};

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
        (
            "FOOTER",
            include_str!("../static/home_footer.html").replace("{{ACCOUNT}}", &account.to_string()),
        ),
    ]);
    Html(render_template(TEMPLATE, &values))
}

pub(super) async fn show(
    app: &App,
    principal: &Principal,
    user: &DbRow,
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
        "<form method=\"post\" action=\"/session\" id=\"sign-out-form\"><input type=\"hidden\" name=\"_method\" value=\"delete\"><button class=\"btn txt-x-small\" type=\"submit\">Sign out of Rizzy on this device</button></form>"
    } else {
        ""
    };
    let passkeys = if self_profile {
        format!(
            "<a class=\"btn txt-x-small\" href=\"/{}/my/passkeys\">Manage passkeys</a>",
            principal.slug
        )
    } else {
        String::new()
    };
    let developer = if self_profile {
        format!(
            "<section class=\"settings__section\"><h2 class=\"divider\">Developer</h2><p>Manage <a href=\"/{}/my/access_tokens\">personal access tokens</a> used with the Rizzy developer API.</p></section>",
            principal.slug
        )
    } else {
        String::new()
    };
    let devices = if self_profile {
        let identity_id: String = user.get("identity_id");
        let transfer = session_transfer::active_token(app, &identity_id).await?;
        let current = if let Some(token) = transfer {
            let url = format!("{}/session/transfers/{token}", base(headers));
            let qr_id = session_transfer::qr_link(app, principal.slug, &url);
            format!(
                "<input type=\"text\" id=\"session_transfer_url\" value=\"{}\" aria-labelledby=\"session_transfer_label\" readonly><button type=\"button\" class=\"button button--secondary\" id=\"show-transfer-qr\">Display auto-login QR code</button><button type=\"button\" class=\"button button--secondary\" id=\"copy-transfer\">Copy auto-login link</button><dialog id=\"transfer-qr-dialog\"><p><strong>Scan this code to instantly log in on another device:</strong></p><img src=\"/{}/qr_codes/{}\" alt=\"QR Code\" width=\"280\" height=\"280\"><form method=\"dialog\"><button class=\"button\">Done</button></form></dialog>",
                escape(&url),
                principal.slug,
                escape(&qr_id)
            )
        } else {
            String::new()
        };
        format!(
            "<section class=\"settings__section\"><h2 class=\"divider\">Devices</h2><p id=\"session_transfer_label\">Single-use link to automatically log in on another device.</p>{current}<form method=\"post\" action=\"/{}/transfer_token\"><button class=\"button\" type=\"submit\">Generate a new auto-login link</button></form></section>",
            principal.slug
        )
    } else {
        String::new()
    };
    let export = if self_profile {
        format!(
            "<section class=\"settings__section\"><h2 class=\"divider\">Export your data</h2><p>Download an archive of your Rizzy data.</p><details><summary>Begin export...</summary><p>This will generate a ZIP archive of all cards you have access to.</p><p>We’ll email you a link to download the file when it’s ready. The link will expire after 24 hours.</p><form method=\"post\" action=\"{path}/data_exports\"><button type=\"submit\">Start export</button></form></details></section>"
        )
    } else {
        String::new()
    };
    let theme = if self_profile {
        "<section class=\"settings__section\"><h2 class=\"divider\">Appearance</h2><div class=\"theme-switcher\"><label class=\"theme-switcher__btn\">Always light<input type=\"radio\" name=\"theme\" value=\"light\"></label><label class=\"theme-switcher__btn\">Always dark<input type=\"radio\" name=\"theme\" value=\"dark\"></label><label class=\"theme-switcher__btn\">Same as OS<input type=\"radio\" name=\"theme\" value=\"auto\"></label></div></section>"
    } else {
        ""
    };
    let card_links = if user.get::<Option<String>, _>("verified_at").is_some() {
        let person = if self_profile {
            "me".to_owned()
        } else {
            name.split_whitespace().next().unwrap_or(&name).to_owned()
        };
        format!(
            "<div class=\"profile-card__links\"><a href=\"/{}/cards?assignee_ids%5B%5D={}&amp;sorted_by=newest\">Which cards are assigned to {}?</a><a href=\"/{}/cards?creator_ids%5B%5D={}&amp;sorted_by=newest\">Which cards were added by {}?</a></div>",
            principal.slug,
            escape(&id),
            escape(&person),
            principal.slug,
            escape(&id),
            escape(&person)
        )
    } else {
        String::new()
    };
    let timeline = if user.get::<Option<String>, _>("verified_at").is_some() {
        events_page::user_fragment(app, principal, user, headers, None).await?
    } else {
        String::new()
    };
    let settings_panel = if self_profile {
        format!(
            "<div class=\"settings__panel panel shadow profile-card profile-card--sections\">{theme}{devices}{developer}{export}</div>"
        )
    } else {
        String::new()
    };
    let content = format!(
        "<div class=\"settings settings--profile\"><div class=\"settings__panel panel shadow txt-align-center profile-card\"><div class=\"profile-card__actions\">{edit}</div><img class=\"profile-avatar\" src=\"{path}/avatar\" alt=\"Profile avatar for {}\"><h1>{}</h1><div class=\"profile-email\">{email_html}</div>{card_links}<div class=\"profile-card__session\">{passkeys}{sign_out}</div></div>{settings_panel}</div>{timeline}",
        escape(&name),
        escape(&name)
    );
    Ok(page(&name, principal.slug, content))
}

pub(super) fn edit(
    principal: &Principal,
    user: &DbRow,
    email: &str,
    error: Option<&str>,
    has_avatar: bool,
) -> Html<String> {
    let id: String = user.get("id");
    let name: String = user.get("name");
    let path = format!("/{}/users/{}", principal.slug, escape(&id));
    let error = error
        .map(|message| format!("<p class=\"form-error\">{}</p>", escape(message)))
        .unwrap_or_default();
    let remove_avatar = if has_avatar {
        "<button type=\"submit\" form=\"avatar-delete-form\" id=\"remove-avatar\" class=\"button button--secondary\">Remove avatar</button>"
    } else {
        ""
    };
    let content = format!(
        "<a href=\"{path}\">← Back to profile</a><div class=\"profile-card\"><h1>Edit your profile</h1>{error}<form class=\"profile-form\" action=\"{path}\" method=\"post\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><div class=\"profile-avatar-controls\"><label class=\"profile-avatar-upload\"><img id=\"avatar-preview\" class=\"profile-avatar\" src=\"{path}/avatar\" alt=\"Profile avatar for {}\"><input id=\"avatar-file\" type=\"file\" name=\"user[avatar]\" accept=\"image/jpeg,image/png,image/gif,image/webp\" data-direct-upload-url=\"/{}/rails/active_storage/direct_uploads\"><span>Choose avatar</span></label>{remove_avatar}</div><label>Name<input type=\"text\" name=\"user[name]\" value=\"{}\" autocomplete=\"name\" required autofocus></label><label>Email address<input type=\"email\" name=\"user[email_address]\" value=\"{}\" readonly></label><a href=\"{path}/email_addresses/new\">Change email</a><div class=\"profile-form__actions\"><button class=\"button\" type=\"submit\">Save changes</button><a href=\"{path}\">Cancel and go back</a></div></form><form id=\"avatar-delete-form\" method=\"post\" action=\"{path}/avatar\"><input type=\"hidden\" name=\"_method\" value=\"delete\"></form></div><script type=\"module\">import {{ start }} from '/assets/lexxy/activestorage.esm.js'; start(); const input=document.getElementById('avatar-file'); const preview=document.getElementById('avatar-preview'); let temporary; input.addEventListener('change',()=>{{if(temporary)URL.revokeObjectURL(temporary); if(input.files[0]){{temporary=URL.createObjectURL(input.files[0]);preview.src=temporary}}}}); document.getElementById('avatar-delete-form')?.addEventListener('submit',async event=>{{event.preventDefault();if(!confirm('Are you sure you want to remove your avatar? This cannot be undone.'))return;const response=await fetch('{path}/avatar',{{method:'DELETE',headers:{{Accept:'application/json'}}}});if(response.ok)location.href='{path}';else alert('Could not remove avatar')}});</script>",
        escape(&name),
        principal.slug,
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
