use axum::{
    http::header,
    response::{Html, IntoResponse, Response},
};
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
    let dom_id = match id {
        "done" => "closed-cards".to_owned(),
        "maybe" | "not-now" => id.to_owned(),
        _ => format!("column_{id}"),
    };
    let mut html = format!(
        "<section class=\"cards cards--{}\" id=\"{}\" data-column-id=\"{}\" data-count=\"{count}\" aria-label=\"{}\"><header class=\"cards__header\"><h2>{}</h2><span class=\"cards__count\">{count}</span></header><div class=\"cards__list\">",
        if id == "maybe" {
            "maybe"
        } else if id == "done" {
            "closed"
        } else if id == "not-now" {
            "on-deck"
        } else {
            "doing"
        },
        escape(&dom_id),
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
                "<a class=\"card{}\" draggable=\"true\" data-card-number=\"{number}\" href=\"/{account}/cards/{number}\"><span class=\"card__number\">#{number}</span><strong class=\"card__title\">{}</strong><span class=\"card__meta\">{}</span></a>",
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

pub(super) async fn drop_column_response(
    app: &App,
    principal: &Principal,
    board_id: &str,
    destination: &str,
    column_id: Option<&str>,
) -> Result<Response> {
    let (id, title, path, add_card) = match destination {
        "closure" => (
            "done".to_owned(),
            "Done".to_owned(),
            "/columns/closed".to_owned(),
            false,
        ),
        "not_now" => (
            "not-now".to_owned(),
            "Not Now".to_owned(),
            "/columns/not_now".to_owned(),
            false,
        ),
        "stream" => (
            "maybe".to_owned(),
            "Maybe?".to_owned(),
            "/columns/stream".to_owned(),
            true,
        ),
        "column" => {
            let id = column_id.ok_or_else(|| super::bad("column_id is required"))?;
            let row = super::checked_column(app, principal, board_id, id).await?;
            (
                id.to_owned(),
                row.get("name"),
                format!("/columns/{id}/cards"),
                false,
            )
        }
        _ => return Err(super::bad("invalid destination")),
    };
    let (predicate, order) = match destination {
        "closure" => ("c.closed=1", "cl.created_at DESC,c.id DESC"),
        "not_now" => (
            "c.closed=0 AND c.postponed=1",
            "c.last_active_at DESC,c.id DESC",
        ),
        "stream" => (
            "c.closed=0 AND c.postponed=0 AND c.column_id IS NULL",
            "c.golden DESC,c.last_active_at DESC,c.id DESC",
        ),
        _ => (
            "c.closed=0 AND c.postponed=0 AND c.column_id=?",
            "c.golden DESC,c.last_active_at DESC,c.id DESC",
        ),
    };
    let count_sql = format!(
        "SELECT COUNT(*) FROM cards c WHERE c.board_id=? AND c.status='published' AND {predicate}"
    );
    let cards_sql = format!(
        "SELECT c.* FROM cards c LEFT JOIN closures cl ON cl.card_id=c.id WHERE c.board_id=? AND c.status='published' AND {predicate} ORDER BY {order} LIMIT 15"
    );
    let (count, cards) = if destination == "column" {
        let count: i64 = sqlx::query_scalar(&count_sql)
            .bind(board_id)
            .bind(&id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
        let cards = sqlx::query(&cards_sql)
            .bind(board_id)
            .bind(&id)
            .fetch_all(&app.db)
            .await
            .map_err(db_err)?;
        (count, cards)
    } else {
        let count: i64 = sqlx::query_scalar(&count_sql)
            .bind(board_id)
            .fetch_one(&app.db)
            .await
            .map_err(db_err)?;
        let cards = sqlx::query(&cards_sql)
            .bind(board_id)
            .fetch_all(&app.db)
            .await
            .map_err(db_err)?;
        (count, cards)
    };
    let target = match destination {
        "closure" => "closed-cards".to_owned(),
        "not_now" => "not-now".to_owned(),
        "stream" => "maybe".to_owned(),
        _ => format!("column_{id}"),
    };
    let content = column_html(
        principal.slug,
        board_id,
        &id,
        &title,
        count,
        Some(&cards),
        &path,
        add_card,
    );
    Ok(([(header::CONTENT_TYPE, "text/vnd.turbo-stream.html; charset=utf-8")],
        format!("<turbo-stream action=\"replace\" method=\"morph\" target=\"{}\"><template>{content}</template></turbo-stream>", escape(&target))).into_response())
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
