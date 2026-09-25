use axum::response::Html;
use sqlx::Row;
use std::collections::HashMap;

use super::{App, Principal, Result, card_page::render_template, db_err, escape};

const TEMPLATE: &str = include_str!("../static/account_settings.html");

pub(super) async fn show(app: &App, p: &Principal, error: Option<&str>) -> Result<Html<String>> {
    let account = sqlx::query("SELECT name,auto_postpone_period_in_days FROM accounts WHERE id=?")
        .bind(&p.account_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let people = sqlx::query("SELECT u.id,u.name,u.role,i.email_address FROM users u JOIN identities i ON i.id=u.identity_id WHERE u.account_id=? AND u.active=1 AND u.role!='system' ORDER BY lower(u.name),u.id")
        .bind(&p.account_id).fetch_all(&app.db).await.map_err(db_err)?;
    let admin = matches!(p.role.as_str(), "admin" | "owner");
    let readonly = if admin { "" } else { " readonly" };
    let disabled = if admin { "" } else { " disabled" };
    let name: String = account.get("name");
    let days: i64 = account.get("auto_postpone_period_in_days");
    let mut people_html = String::new();
    for user in people {
        let id: String = user.get("id");
        let user_name: String = user.get("name");
        let role: String = user.get("role");
        let email: String = user.get("email_address");
        let can_administer = admin && id != p.user_id && role != "owner";
        let controls_disabled = if can_administer { "" } else { " disabled" };
        let checked = if role == "admin" { " checked" } else { "" };
        let role_name = match role.as_str() {
            "admin" => "Administrator",
            "owner" => "Owner",
            _ => "Member",
        };
        let role_control_name = if role == "owner" {
            "Owner"
        } else {
            "Administrator"
        };
        let path = format!("/{}/users/{}", p.slug, escape(&id));
        people_html.push_str(&format!(
            "<li class=\"person\" data-filter-item><a class=\"person__profile\" href=\"{path}\"><img src=\"{path}/avatar\" alt=\"\"><span><strong>{}</strong><small>{}</small></span></a><span class=\"person__controls\"><label title=\"{role_name}\"><input class=\"role-toggle\" type=\"checkbox\" aria-label=\"Role: {role_name}\" data-user=\"{}\"{checked}{controls_disabled}>{role_control_name}</label><button class=\"remove-user\" type=\"button\" aria-label=\"Remove {} from the account\" data-user=\"{}\"{controls_disabled}>Remove</button></span></li>",
            escape(&user_name), escape(&email), escape(&id), escape(&user_name), escape(&id)
        ));
    }
    let mut options = String::new();
    for value in [3, 7, 30, 90, 365, 11] {
        let checked = if value == days { " checked" } else { "" };
        options.push_str(&format!("<label><input type=\"radio\" name=\"entropy[auto_postpone_period_in_days]\" value=\"{value}\"{checked}{disabled}><span>{value}</span></label>"));
    }
    let admin_note = if admin {
        ""
    } else {
        "<p class=\"subtle\">Only admins can change these settings</p>"
    };
    let name_error = error
        .map(|message| format!("<p class=\"error\">{}</p>", escape(message)))
        .unwrap_or_default();
    let save = if admin {
        "<button class=\"save-name\" type=\"submit\" aria-label=\"Save changes\">→</button>"
    } else {
        ""
    };
    let export = if admin {
        format!(
            "<section class=\"section\"><h2>Export account data</h2><p>Download a complete archive of all account data.</p><button type=\"button\" id=\"begin-export\">Begin export...</button><dialog id=\"export-dialog\"><h2>Export all account data</h2><p>This will generate a ZIP archive of all data in this account including all boards, cards, users, and settings.</p><p>We’ll email you a link to download the file when it’s ready. The link will expire after 24 hours.</p><form method=\"post\" action=\"/{}/account/exports\"><button type=\"submit\">Start export</button><button type=\"button\" id=\"cancel-export\">Cancel</button></form></dialog></section>",
            p.slug
        )
    } else {
        String::new()
    };
    let content = format!(
        "<h1>Account Settings</h1>{admin_note}<div class=\"settings\"><div class=\"panel\"><section class=\"section section--name\">{name_error}<form method=\"post\" action=\"/{}/account/settings\"><input type=\"hidden\" name=\"_method\" value=\"put\"><input class=\"account-name\" type=\"text\" name=\"account[name]\" value=\"{}\" placeholder=\"Account name…\" required{readonly}>{save}</form></section><section class=\"section\"><h2>People on this account</h2><input id=\"people-filter\" type=\"search\" placeholder=\"Filter…\" aria-label=\"Filter people\"><ul class=\"people\" role=\"listbox\">{people_html}</ul><a class=\"invite\" href=\"/{}/account/join_code\">＋ Invite people</a></section></div><div class=\"panel\"><section class=\"section\"><h2>Auto close</h2><p>Rizzy doesn’t let stale cards stick around forever. Cards automatically move to “Not Now” if there is no activity for a specific period of time. <em>This is the default, global setting — you can override it on each board.</em></p><form id=\"entropy-form\" method=\"post\" action=\"/{}/account/entropy\"><input type=\"hidden\" name=\"_method\" value=\"put\"><div class=\"entropy-options\" role=\"radiogroup\" aria-label=\"Days until auto-close\">{options}</div><div class=\"days-label\">Days until auto-close</div></form></section>{export}</div></div>",
        p.slug,
        escape(&name),
        p.slug,
        p.slug
    );
    let values = HashMap::from([("ACCOUNT", p.slug.to_string()), ("CONTENT", content)]);
    Ok(Html(render_template(TEMPLATE, &values)))
}
