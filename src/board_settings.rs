use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use sqlx::Row;
use std::collections::HashMap;

use super::{
    Result, Shared, board, card_page::render_template, db_err, err, escape, principal,
    signed_in_shell_html,
};

const TEMPLATE: &str = include_str!("../static/board_settings.html");
const SCRIPT: &str = include_str!("../static/board_settings.js");
const STYLES: &str = include_str!("../static/board_settings.css");

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
        ("SCRIPT", SCRIPT.to_owned()),
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
        users.push_str(&format!("<li class=\"person\" data-board-access-person><a href=\"/{}/users/{}\" class=\"person-link\"><figure class=\"avatar\" aria-hidden=\"true\">{}</figure><span><strong>{}</strong><small>{}</small></span></a><hr aria-hidden=\"true\"><label class=\"person-switch switch\"><input type=\"checkbox\" name=\"user_ids[]\" value=\"{}\" aria-label=\"Give {} access\"{checked}{disabled}{me}><span class=\"switch__btn\" aria-hidden=\"true\"></span></label><span class=\"person-selected-mark\" aria-hidden=\"true\">✓</span></li>",p.slug,escape(&user_id),escape(&initials),escape(&user_name),escape(&user_email),escape(&user_id),escape(&user_name)));
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
    let entropy = entropy_frame(&board_path, &id, days, disabled);
    let publication = publication_frame(
        &board_path,
        &id,
        published,
        &description,
        &public_link,
        disabled,
    );
    let admin_note = if admin {
        String::new()
    } else {
        "<div class=\"muted\">Only admins can change these settings</div>".to_owned()
    };
    let delete_button = if admin {
        "<div class=\"delete-control\"><button class=\"btn txt-negative\" id=\"delete-board\" type=\"button\">Delete this board</button><dialog id=\"board-delete-dialog\" class=\"dialog panel\"><h3>Delete this board?</h3><p>Are you sure you want to permanently delete this board and all the cards on it? This can't be undone.</p><div class=\"dialog__actions\"><button id=\"cancel-delete-board\" type=\"button\" class=\"btn\">Cancel</button><form action=\"BOARD_PATH\" method=\"post\"><input type=\"hidden\" name=\"_method\" value=\"delete\"><button type=\"submit\" class=\"btn txt-negative\">Delete board</button></form></div></dialog></div>"
            .replace("BOARD_PATH", &board_path)
    } else {
        String::new()
    };
    let content = format!(
        "<section class=\"settings board-settings\" data-board-path=\"{board_path}\"><div class=\"settings__panel settings__panel--users panel shadow\"><form class=\"display-contents\" id=\"settings-form\" action=\"{board_path}\" method=\"post\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><label class=\"board-name\"><strong><input class=\"input full-width txt-medium\" type=\"text\" name=\"board[name]\" value=\"{}\" placeholder=\"Board name…\" required{disabled}></strong></label><h2 class=\"divider\">Who can access this board?</h2><div class=\"settings__user-filter\"><ul class=\"access-menu\"><li class=\"person everyone\"><figure class=\"avatar\" aria-hidden=\"true\">◎</figure><strong>Everyone</strong><hr aria-hidden=\"true\"><label class=\"switch\"><input type=\"hidden\" name=\"board[all_access]\" value=\"0\"><input id=\"board-all-access\" type=\"checkbox\" name=\"board[all_access]\" value=\"1\"{}{disabled}><span class=\"switch__btn\" aria-hidden=\"true\"></span><span class=\"for-screen-reader\">Give everyone access to this board</span></label></li></ul><input id=\"board-access-filter\" class=\"input input--transparent full-width txt-small\" type=\"search\" placeholder=\"Filter…\" aria-label=\"Filter people\" autocomplete=\"off\"><div id=\"board-access-controls\"{}><div class=\"access-bulk\"><button type=\"button\" name=\"button\" id=\"access-select-all\"{disabled}>Select all</button><span aria-hidden=\"true\">·</span><button type=\"button\" name=\"button\" id=\"access-select-none\"{disabled}>Select none</button></div></div><ul id=\"board-access-people\" class=\"settings__scrollable-list\">{users}</ul></div><button class=\"btn btn--link center\" id=\"log_in\" type=\"submit\"{disabled}>Save changes</button><a href=\"{board_path}\" hidden>Cancel and go back</a></form></div><div class=\"settings__panel panel shadow\"><div class=\"settings__section\"><h2 class=\"divider\">Auto close</h2><p>Rizzy doesn’t let stale cards stick around forever. Cards automatically move to “Not now” if no one updates, comments, or moves a card for…</p>{entropy}</div><div class=\"settings__section\"><h2 class=\"divider\">Public link</h2><p>Turn on the Public link to share this board with anyone in the world. They won’t need to log in and they won’t be able to see anything else in Rizzy.</p>{publication}</div>{delete_button}</div></section><script>{SCRIPT}</script>",
        escape(&name),
        if all { " checked" } else { "" },
        if all { " hidden" } else { "" },
    );
    let header = format!(
        "<div class=\"header__actions header__actions--start\"><a href=\"{board_path}\" class=\"header__back\">Back to {}</a></div><h1 class=\"header__title\" data-bridge--title-target=\"header\">Board Settings{admin_note}</h1>",
        escape(&name)
    );
    Ok(signed_in_shell_html(
        p.slug,
        "Board Settings",
        &content,
        "",
        STYLES,
        &header,
    ))
}

fn entropy_frame(board_path: &str, id: &str, days: i64, disabled: &str) -> String {
    let mut options = String::new();
    let choices = [3, 7, 30, 90, 365, 11];
    for choice in choices {
        options.push_str(&format!("<label><input type=\"radio\" name=\"board[auto_postpone_period_in_days]\" value=\"{choice}\"{}{}><span>{choice}</span></label>",if choice==days { " checked" } else { "" },disabled));
    }
    let slider = choices
        .iter()
        .position(|choice| *choice == days)
        .unwrap_or(2);
    let angle = -22 + (slider as i32 - 2) * 35;
    format!(
        "<turbo-frame id=\"entropy_board_{}\"><form id=\"board-entropy-form\" action=\"{board_path}/entropy\" method=\"post\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><div class=\"options\" role=\"radiogroup\" style=\"--knob-angle:{angle}deg\">{options}<input type=\"range\" name=\"board[slider]\" min=\"0\" max=\"5\" value=\"{slider}\" aria-hidden=\"true\"{disabled}></div><div class=\"knob__label\">DAYS</div></form></turbo-frame>",
        escape(id)
    )
}

fn publication_frame(
    board_path: &str,
    id: &str,
    published: bool,
    description: &str,
    public_link: &str,
    disabled: &str,
) -> String {
    let method = if published {
        "<input type=\"hidden\" name=\"_method\" value=\"delete\">"
    } else {
        ""
    };
    let checked = if published { " checked" } else { "" };
    let note = if published {
        format!(
            "{public_link}<form action=\"{board_path}\" method=\"post\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><label class=\"field\"><span>Public page description</span><textarea name=\"board[public_description]\"{disabled}>{}</textarea></label><button class=\"button\" type=\"submit\"{disabled}>Save changes</button></form>",
            escape(description)
        )
    } else {
        String::new()
    };
    format!(
        "<turbo-frame id=\"publication_board_{}\"><form id=\"board-publication-form\" action=\"{board_path}/publication\" method=\"post\">{method}<span class=\"publication-icon\" aria-hidden=\"true\"><svg viewBox=\"0 0 24 24\"><rect x=\"4\" y=\"10\" width=\"16\" height=\"12\" rx=\"2\" fill=\"currentColor\"/><path d=\"M7 10V7a5 5 0 0 1 10 0v3\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"/></svg></span><label class=\"switch\"><input type=\"hidden\" name=\"published\" value=\"0\"><input id=\"publication-toggle\" type=\"checkbox\" name=\"published\" value=\"1\"{checked}{disabled}><span class=\"switch__btn\" aria-hidden=\"true\"></span><span class=\"for-screen-reader\">Turn {} the public link</span></label><span class=\"publication-icon\" aria-hidden=\"true\"><svg viewBox=\"0 0 24 24\" fill=\"none\" stroke=\"currentColor\" stroke-width=\"2\"><circle cx=\"12\" cy=\"12\" r=\"9\"/><path d=\"M3 12h18M12 3c-5 5-5 13 0 18M12 3c5 5 5 13 0 18\"/></svg></span></form>{note}</turbo-frame>",
        escape(id),
        if published { "off" } else { "on" }
    )
}

pub(super) async fn stream(
    app: &Shared,
    account: &str,
    id: &str,
    headers: &HeaderMap,
    kind: &str,
) -> Result<Response> {
    let p = principal(app, headers, account).await?;
    let b = board(app, &p, id).await?;
    let board_path = format!("/{}/boards/{}", p.slug, escape(id));
    let admin = p.admin(&b.get::<String, _>("creator_id")).is_ok();
    let disabled = if admin { "" } else { " disabled" };
    let frame = if kind == "entropy" {
        entropy_frame(
            &board_path,
            id,
            b.get("auto_postpone_period_in_days"),
            disabled,
        )
    } else {
        let key: Option<String> = b.get("publication_key");
        let link = key
            .as_ref()
            .map(|key| {
                format!(
                    "<p><a href=\"/{}/public/boards/{}\">Open public board</a></p>",
                    p.slug,
                    escape(key)
                )
            })
            .unwrap_or_default();
        publication_frame(
            &board_path,
            id,
            key.is_some(),
            b.get("public_description"),
            &link,
            disabled,
        )
    };
    let target = format!("{kind}_board_{}", escape(id));
    let html = format!(
        "<turbo-stream action=\"replace\" target=\"{target}\"><template>{frame}</template></turbo-stream><turbo-stream action=\"replace\" target=\"flash\"><template><div id=\"flash\">Saved</div></template></turbo-stream>"
    );
    Ok((
        [(
            header::CONTENT_TYPE,
            "text/vnd.turbo-stream.html; charset=utf-8",
        )],
        html,
    )
        .into_response())
}
