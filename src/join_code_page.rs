use axum::response::Html;
use std::collections::HashMap;

use super::{Principal, card_page::render_template, escape};

const TEMPLATE: &str = include_str!("../static/join_code.html");

fn page(p: &Principal, title: &str, content: String) -> Html<String> {
    let values = HashMap::from([
        ("ACCOUNT", p.slug.to_string()),
        ("TITLE", title.to_owned()),
        ("CONTENT", content),
    ]);
    Html(render_template(TEMPLATE, &values))
}

pub(super) fn show(p: &Principal, url: &str, count: i64, limit: i64) -> Html<String> {
    let admin = matches!(p.role.as_str(), "owner" | "admin");
    let remaining = if count < limit {
        (limit - count).to_string()
    } else {
        "none".into()
    };
    let reset = if admin {
        format!(
            "<button type=\"button\" id=\"reset-link\" data-url=\"/{}/account/join_code\" title=\"Generate a new code\">↻ <span class=\"sr-only\">Generate a new code</span></button>",
            p.slug
        )
    } else {
        String::new()
    };
    let limit_link = if admin {
        format!(
            "<a href=\"/{}/account/join_code/edit\">Change limit</a>",
            p.slug
        )
    } else {
        String::new()
    };
    let content = format!(
        "<a href=\"/{}/account/settings\">← Account Settings</a><article class=\"panel\"><h1>Add people</h1><p>Share the link below to invite people to this account</p><div class=\"link-row\"><input id=\"invite-url\" type=\"text\" value=\"{}\" readonly aria-label=\"Invite link\">{reset}</div><div class=\"actions\"><button type=\"button\" id=\"copy-link\">Copy invite link</button></div><footer>This code has been used {count}/{limit} times ({remaining} remaining) {limit_link}</footer></article>",
        p.slug,
        escape(url)
    );
    page(p, "Add people", content)
}

pub(super) fn edit(p: &Principal, count: i64, limit: i64, error: Option<&str>) -> Html<String> {
    let error = error
        .map(|message| format!("<p class=\"error txt-negative\">{}</p>", escape(message)))
        .unwrap_or_default();
    let content = format!(
        "<a href=\"/{}/account/join_code\">← Invite link</a><article class=\"panel\"><h1>Change usage limit</h1><p>How many times can this link be used to join the account?</p><form method=\"post\" action=\"/{}/account/join_code\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><input class=\"limit\" type=\"number\" name=\"account_join_code[usage_limit]\" value=\"{limit}\" min=\"0\" max=\"10000000000\" required autofocus>{error}<p class=\"subtle\">This code has been used {count}/{limit} times.</p><button type=\"submit\">Save changes</button></form></article>",
        p.slug, p.slug
    );
    page(p, "Change usage limit", content)
}
