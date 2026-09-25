use axum::response::Html;
use sqlx::{Row, sqlite::SqliteRow};

use super::escape;

fn shell(title: &str, back: &str, content: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} | Rizzy</title><style>:root{{font:16px/1.5 system-ui,sans-serif;color:#f5f0e8;background:#1b1b1b}}*{{box-sizing:border-box}}body{{margin:0}}a{{color:#e7bd61}}.site{{display:flex;gap:20px;align-items:center;border-bottom:1px solid #444;padding:16px 4vw}}.site strong{{font-size:26px}}main{{max-width:730px;margin:50px auto;padding:0 20px}}.panel{{background:#292826;border:1px solid #514d45;border-radius:14px;padding:28px}}h1{{font-size:28px}}table{{width:100%;border-collapse:collapse}}th,td{{padding:12px 8px;text-align:left;border-bottom:1px solid #514d45}}input,select{{font:inherit;background:#393631;color:#fff;border:1px solid #777;border-radius:6px;padding:9px;width:100%}}button,.button{{font:inherit;background:#493a20;color:#fff;border:1px solid #9d844f;border-radius:8px;padding:9px 13px;cursor:pointer;display:inline-block;text-decoration:none}}label{{display:block;margin:16px 0}}.muted{{color:#bbb}}.actions{{display:flex;gap:10px;align-items:center}}.danger{{color:#ffb2aa;background:transparent;border:0}}@media(max-width:650px){{th:nth-child(3),td:nth-child(3){{display:none}}}}</style></head><body><header class=\"site\"><strong>Rizzy</strong><a href=\"{}\">← Back</a></header><main><h1>{}</h1><section class=\"panel\">{}</section></main><script>document.querySelectorAll('[data-confirm-revoke]').forEach(form=>form.addEventListener('submit',event=>{{if(!confirm('Are you sure you want to permanently revoke this access token?'))event.preventDefault()}}));document.getElementById('copy-token')?.addEventListener('click',async event=>{{await navigator.clipboard.writeText(document.getElementById('new-token').value);event.currentTarget.textContent='Copied'}})</script></body></html>",
        escape(title),
        escape(back),
        escape(title),
        content
    ))
}

pub(super) fn index(rows: &[SqliteRow], root: &str, back: &str) -> Html<String> {
    let mut content = String::new();
    if rows.is_empty() {
        content.push_str("<p>Personal access tokens can be used like a password to access the Rizzy developer API. You can have as many tokens as you need and revoke access to each one at any time.</p>");
    } else {
        content.push_str("<p>Tokens you have generated that can be used to access the Rizzy API.</p><table class=\"access-tokens\"><thead><tr><th>Description</th><th>Permission</th><th>Created</th><th></th></tr></thead><tbody>");
        for row in rows {
            let id: String = row.get("id");
            let description: String = row.get("description");
            let permission: String = row.get("permission");
            let created: String = row.get("created_at");
            content.push_str(&format!("<tr><td><strong>{}</strong></td><td>{}</td><td><time datetime=\"{}\">{}</time></td><td><form method=\"post\" action=\"{root}/my/access_tokens/{}\" data-confirm-revoke><input type=\"hidden\" name=\"_method\" value=\"delete\"><button class=\"danger\" type=\"submit\" aria-label=\"Revoke {}\">Revoke</button></form></td></tr>",escape(&description),if permission=="write"{"Write"}else{"Read"},escape(&created),escape(&created),escape(&id),escape(&description)));
        }
        content.push_str("</tbody></table>");
    }
    content.push_str(&format!("<p><a class=\"button\" href=\"{root}/my/access_tokens/new\">Generate a new access token</a></p>"));
    shell("Personal access tokens", back, &content)
}

pub(super) fn new(root: &str) -> Html<String> {
    shell(
        "Generate a personal access token",
        &format!("{root}/my/access_tokens"),
        &format!(
            "<form method=\"post\" action=\"{root}/my/access_tokens\"><label><strong>Access token description</strong><input name=\"access_token[description]\" placeholder=\"e.g. Github\" required autofocus></label><label><strong>Permission</strong><select name=\"access_token[permission]\"><option value=\"read\">Read</option><option value=\"write\">Read + Write</option></select></label><button type=\"submit\">Generate access token</button></form>"
        ),
    )
}

pub(super) fn show(root: &str, description: &str, permission: &str, token: &str) -> Html<String> {
    shell(
        "New personal access token",
        &format!("{root}/my/access_tokens"),
        &format!(
            "<label><strong>{} ({})</strong><input id=\"new-token\" type=\"text\" value=\"{}\" readonly></label><p class=\"muted\">Be sure to save this access token now because you won’t be able to see it again.</p><button id=\"copy-token\" type=\"button\">Copy access token</button>",
            escape(description),
            if permission == "write" {
                "Read + Write"
            } else {
                "Read"
            },
            escape(token)
        ),
    )
}
