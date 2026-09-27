use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::Html,
};
use sqlx::Row;
use std::collections::HashMap;

use super::{Result, Shared, board, card_page::render_template, db_err, err, escape, principal};

const TEMPLATE: &str = include_str!("../static/board_settings.html");

pub(super) async fn new(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Html<String>> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let content = format!(
        "<h1>Create a new board</h1><div class=\"panel\"><form class=\"stack\" action=\"/{}/boards\" method=\"post\"><label class=\"field\"><span>Board name</span><input type=\"text\" name=\"board[name]\" placeholder=\"Name it…\" autocomplete=\"off\" required autofocus></label><div class=\"actions\"><button class=\"button\" type=\"submit\">Create board</button><a href=\"/{}/\">Cancel and go back</a></div></form></div>",
        p.slug, p.slug
    );
    let values = HashMap::from([
        ("TITLE", "Create a new board".to_owned()),
        ("HOME_PATH", format!("/{}/", p.slug)),
        ("BOARD_PATH", String::new()),
        ("BOARD_LINK", String::new()),
        ("CONTENT", content),
    ]);
    Ok(Html(render_template(TEMPLATE, &values)))
}

pub(super) async fn edit(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Html<String>> {
    let p = principal(&app, &headers, &account).await?;
    let b = board(&app, &p, &id).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let admin = p.admin(&b.get::<String, _>("creator_id")).is_ok();
    let disabled = if admin { "" } else { " disabled" };
    let board_path = format!("/{}/boards/{}", p.slug, escape(&id));
    let name: String = b.get("name");
    let all = b.get::<i64, _>("all_access") != 0;
    let days: i64 = b.get("auto_postpone_period_in_days");
    let description: String = b.get("public_description");
    let publication_key: Option<String> = b.get("publication_key");
    let people = sqlx::query("SELECT u.id,u.name,i.email_address,(x.user_id IS NOT NULL) AS selected FROM users u JOIN identities i ON i.id=u.identity_id LEFT JOIN accesses x ON x.user_id=u.id AND x.board_id=? WHERE u.account_id=? AND u.active=1 AND u.role!='system' ORDER BY selected DESC,lower(u.name),u.id")
        .bind(&id).bind(&p.account_id).fetch_all(&app.db).await.map_err(db_err)?;
    let mut users = String::new();
    for user in people {
        let user_id: String = user.get("id");
        let user_name: String = user.get("name");
        let user_email: String = user.get("email_address");
        let initials = user_name
            .split_whitespace()
            .take(2)
            .filter_map(|part| part.chars().next())
            .collect::<String>()
            .to_uppercase();
        let checked = if user.get::<i64, _>("selected") != 0 {
            " checked"
        } else {
            ""
        };
        let me = if user_id == p.user_id {
            " id=\"current-user-access\""
        } else {
            ""
        };
        users.push_str(&format!("<div class=\"person\" data-board-access-person><a href=\"/{}/users/{}\" class=\"person-link\"><span class=\"person-avatar\" aria-hidden=\"true\">{}</span><span><strong>{}</strong><small>{}</small></span></a><label class=\"person-switch\"><input type=\"checkbox\" name=\"user_ids[]\" value=\"{}\" aria-label=\"Give {} access\"{checked}{disabled}{me}></label></div>",p.slug,escape(&user_id),escape(&initials),escape(&user_name),escape(&user_email),escape(&user_id),escape(&user_name)));
    }
    let mut entropy = String::new();
    for choice in [3, 7, 30, 90, 365, 11] {
        entropy.push_str(&format!("<label><input type=\"radio\" name=\"board[auto_postpone_period_in_days]\" value=\"{choice}\"{}{}>{choice} days</label>",if choice==days { " checked" } else { "" },disabled));
    }
    let published = publication_key.is_some();
    let public_link = publication_key
        .as_ref()
        .map(|key| {
            format!(
                "<p><a href=\"/{}/public/boards/{}\">Open public board</a></p>",
                p.slug,
                escape(key)
            )
        })
        .unwrap_or_default();
    let public_note = if admin {
        format!(
            "<label class=\"field\"><span>Public page description</span><textarea name=\"board[public_description]\">{}</textarea></label>",
            escape(&description)
        )
    } else {
        String::new()
    };
    let admin_note = if admin {
        String::new()
    } else {
        "<p class=\"muted\">Only admins can change these settings.</p>".to_owned()
    };
    let delete_button = if admin {
        "<button class=\"button button--danger\" id=\"delete-board\" type=\"button\">Delete this board</button>"
    } else {
        ""
    };
    let content = format!(
        "<h1>Board Settings</h1>{admin_note}<div class=\"panel\"><form class=\"stack\" id=\"settings-form\" action=\"{board_path}\" method=\"post\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><label class=\"field\"><span>Board name</span><input type=\"text\" name=\"board[name]\" value=\"{}\" required{disabled}></label><h2>Who can access this board?</h2><label class=\"person\"><input type=\"hidden\" name=\"board[all_access]\" value=\"0\"><input id=\"board-all-access\" type=\"checkbox\" name=\"board[all_access]\" value=\"1\"{}{disabled}><strong>Everyone</strong></label><input id=\"board-access-filter\" type=\"search\" placeholder=\"Filter…\" aria-label=\"Filter people\" autocomplete=\"off\"><div id=\"board-access-controls\"{}><div class=\"access-bulk\"><button type=\"button\" id=\"access-select-all\"{disabled}>Select all</button><span aria-hidden=\"true\">·</span><button type=\"button\" id=\"access-select-none\"{disabled}>Select none</button></div></div><div id=\"board-access-people\">{users}</div>{public_note}<div class=\"actions\"><button class=\"button\" type=\"submit\"{disabled}>Save changes</button><a href=\"{board_path}\">Cancel and go back</a></div></form></div><div class=\"panel\"><h2>Auto close</h2><p class=\"muted\">Cards automatically move to Not Now after a period of inactivity.</p><div class=\"options\">{entropy}</div></div><div class=\"panel\"><h2>Public link</h2><p class=\"muted\">Let anyone view this board without signing in.</p><label class=\"person\"><input id=\"publication-toggle\" type=\"checkbox\"{}{disabled}>Public link enabled</label>{public_link}</div><div class=\"panel\">{delete_button}</div>",
        escape(&name),
        if all { " checked" } else { "" },
        if all { " hidden" } else { "" },
        if published { " checked" } else { "" }
    );
    let values = HashMap::from([
        ("TITLE", "Board Settings".to_owned()),
        ("HOME_PATH", format!("/{}/", p.slug)),
        ("BOARD_PATH", board_path.clone()),
        (
            "BOARD_LINK",
            format!("<a href=\"{board_path}\">Back to board</a>"),
        ),
        ("CONTENT", content),
    ]);
    Ok(Html(render_template(TEMPLATE, &values)))
}
