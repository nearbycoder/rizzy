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

pub(super) fn show(
    p: &Principal,
    url: &str,
    qr_url: &str,
    account_name: &str,
    count: i64,
    limit: i64,
) -> Html<String> {
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
        "<a href=\"/{}/account/settings\">← Account Settings</a><article class=\"panel\"><h1>Add people</h1><p>Share the link below to invite people to this account</p><div class=\"link-row\"><input id=\"invite-url\" type=\"text\" value=\"{}\" readonly aria-label=\"Invite link\">{reset}</div><div class=\"actions\"><button type=\"button\" id=\"copy-link\">Copy invite link</button><button type=\"button\" id=\"show-invite-qr\">Get QR code</button></div><dialog id=\"invite-qr-dialog\"><p><strong>Scan this code to join {}:</strong></p><img class=\"qr-code center\" src=\"{}\" alt=\"QR Code\" width=\"280\" height=\"280\"><form method=\"dialog\"><button type=\"submit\">Done</button></form></dialog><footer>This code has been used {count}/{limit} times ({remaining} remaining) {limit_link}</footer></article>",
        p.slug,
        escape(url),
        escape(account_name),
        escape(qr_url),
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

pub(super) fn inactive(name: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>That code is all used up | Rizzy</title></head><body style=\"font:16px/1.5 system-ui,sans-serif;background:#f7f5f0;color:#252322\"><main style=\"max-width:540px;margin:12vh auto;padding:30px;background:white;border-radius:16px;text-align:center\"><h1>That code is all used up</h1><p>Ask someone from {} to send you a new link or increase the limit.</p><a href=\"/\">OK</a></main></body></html>",
        escape(name)
    ))
}
