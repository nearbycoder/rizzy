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
        "<section class=\"cards cards--{}\" id=\"{}\" aria-label=\"{}\"><header class=\"cards__header\"><h2>{}</h2><span class=\"cards__count\">{count}</span></header><div class=\"cards__list\">",
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
    }
    html.push_str("</div>");
    if count > 15 {
        html.push_str(&format!(
            "<button class=\"cards__more button button--secondary\" type=\"button\" data-path=\"/{account}/boards/{}{}\" data-page=\"2\">Load more</button>",
            escape(board_id),
            escape(path),
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
    // Rank each visible card once, keeping only the first page for every column.
    // Done uses its closure timestamp; active columns put golden cards first.
    let cards = sqlx::query(
        "WITH scoped AS (SELECT c.*,CASE WHEN c.closed=1 THEN 'done' WHEN c.postponed=1 THEN 'not-now' ELSE COALESCE(c.column_id,'maybe') END AS group_key,cl.created_at AS closed_at FROM cards c LEFT JOIN closures cl ON cl.card_id=c.id WHERE c.board_id=? AND c.status='published'), ranked AS (SELECT scoped.*,COUNT(*) OVER (PARTITION BY group_key) AS group_count,ROW_NUMBER() OVER (PARTITION BY group_key ORDER BY CASE WHEN closed=1 THEN closed_at END DESC,CASE WHEN closed=0 AND postponed=0 THEN golden END DESC,last_active_at DESC,id DESC) AS group_rank FROM scoped) SELECT * FROM ranked WHERE group_rank<=15 ORDER BY group_key,group_rank",
    )
    .bind(&board_id)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let mut grouped: HashMap<String, Vec<SqliteRow>> = HashMap::new();
    let mut counts: HashMap<String, i64> = HashMap::new();
    for card in cards {
        let key: String = card.get("group_key");
        counts.insert(key.clone(), card.get("group_count"));
        grouped.entry(key).or_default().push(card);
    }
    let account = principal.slug;
    let mut column_markup = String::new();
    column_markup.push_str(&column_html(
        account,
        &board_id,
        "not-now",
        "Not Now",
        *counts.get("not-now").unwrap_or(&0),
        grouped.get("not-now"),
        "/columns/not_now",
        false,
    ));
    column_markup.push_str(&column_html(
        account,
        &board_id,
        "maybe",
        "Maybe?",
        *counts.get("maybe").unwrap_or(&0),
        grouped.get("maybe"),
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
            grouped.get(&id),
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
        grouped.get("done"),
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
