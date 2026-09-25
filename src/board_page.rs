use axum::response::Html;
use sqlx::{Row, sqlite::SqliteRow};
use std::collections::HashMap;

use super::{App, Principal, Result, card_page::render_template, db_err, escape};

const TEMPLATE: &str = include_str!("../static/board.html");

fn column_html(
    account: i64,
    board_id: &str,
    id: &str,
    title: &str,
    count: i64,
    cards: Option<&Vec<SqliteRow>>,
    path: &str,
    add_card: bool,
) -> String {
    let mut html = format!(
        "<section class=\"cards cards--{}\" id=\"{}\" data-count=\"{count}\" aria-label=\"{}\"><header class=\"cards__header\"><h2>{}</h2><span class=\"cards__count\">{count}</span></header><div class=\"cards__list\">",
        if id == "maybe" {
            "maybe"
        } else if id == "done" {
            "closed"
        } else if id == "not-now" {
            "on-deck"
        } else {
            "doing"
        },
        escape(id),
        escape(title),
        escape(title),
    );
    if add_card {
        html.push_str(&format!(
            "<div class=\"board-tools\"><form id=\"add-card\" action=\"/{account}/boards/{}/cards\" method=\"post\"><button class=\"button\" type=\"submit\">Add a card</button></form></div>",
            escape(board_id)
        ));
    }
    if let Some(cards) = cards {
        for card in cards {
            let number: i64 = card.get("number");
            let title: String = card.get("title");
            let golden = card.get::<i64, _>("golden") != 0;
            html.push_str(&format!(
                "<a class=\"card{}\" href=\"/{account}/cards/{number}\"><span class=\"card__number\">#{number}</span><strong class=\"card__title\">{}</strong><span class=\"card__meta\">{}</span></a>",
                if golden { " card--golden" } else { "" },
                escape(&title),
                if id == "done" { "Done" } else if id == "not-now" { "Not Now" } else { "Open" },
            ));
        }
    }
    if count == 0 {
        html.push_str("<p class=\"cards__empty\">No cards here yet.</p>");
    } else if cards.is_none() {
        html.push_str("<p class=\"cards__empty\" data-loading>Loading cards…</p>");
    }
    html.push_str("</div>");
    let lazy = !add_card && cards.is_none() && count > 0;
    if lazy || count > 15 {
        html.push_str(&format!(
            "<button class=\"cards__more button button--secondary\" type=\"button\" data-path=\"/{account}/boards/{}{}\" data-page=\"{}\">{}</button>",
            escape(board_id),
            escape(path),
            if lazy { 1 } else { 2 },
            if lazy { "Load cards" } else { "Load more" },
        ));
    }
    html.push_str("</section>");
    html
}

pub(super) async fn show(
    app: &App,
    principal: &Principal,
    board: &SqliteRow,
) -> Result<Html<String>> {
    let board_id: String = board.get("id");
    let board_name: String = board.get("name");
    let columns = sqlx::query("SELECT id,name FROM columns WHERE board_id=? ORDER BY position,id")
        .bind(&board_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    // Fizzy renders the Maybe stream immediately and loads the other columns
    // when their frames enter view. Keep that first response bounded by 15 cards.
    let count_rows = sqlx::query("SELECT CASE WHEN closed=1 THEN 'done' WHEN postponed=1 THEN 'not-now' ELSE COALESCE(column_id,'maybe') END AS group_key,COUNT(*) AS total FROM cards WHERE board_id=? AND status='published' GROUP BY group_key")
        .bind(&board_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let mut counts: HashMap<String, i64> = HashMap::new();
    for row in count_rows {
        counts.insert(row.get("group_key"), row.get("total"));
    }
    let maybe = sqlx::query("SELECT * FROM cards WHERE board_id=? AND column_id IS NULL AND status='published' AND closed=0 AND postponed=0 ORDER BY golden DESC,last_active_at DESC,id DESC LIMIT 15")
        .bind(&board_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let account = principal.slug;
    let mut column_markup = String::new();
    column_markup.push_str(&column_html(
        account,
        &board_id,
        "not-now",
        "Not Now",
        *counts.get("not-now").unwrap_or(&0),
        None,
        "/columns/not_now",
        false,
    ));
    column_markup.push_str(&column_html(
        account,
        &board_id,
        "maybe",
        "Maybe?",
        *counts.get("maybe").unwrap_or(&0),
        Some(&maybe),
        "/columns/stream",
        true,
    ));
    for column in columns {
        let id: String = column.get("id");
        let name: String = column.get("name");
        column_markup.push_str(&column_html(
            account,
            &board_id,
            &id,
            &name,
            *counts.get(&id).unwrap_or(&0),
            None,
            &format!("/columns/{id}/cards"),
            false,
        ));
    }
    column_markup.push_str(&column_html(
        account,
        &board_id,
        "done",
        "Done",
        *counts.get("done").unwrap_or(&0),
        None,
        "/columns/closed",
        false,
    ));
    let values = HashMap::from([
        ("ACCOUNT", account.to_string()),
        ("BOARD_ID", escape(&board_id)),
        ("BOARD_NAME", escape(&board_name)),
        ("COLUMNS", column_markup),
    ]);
    Ok(Html(render_template(TEMPLATE, &values)))
}
