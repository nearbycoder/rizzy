use axum::{
    http::header,
    response::{Html, IntoResponse, Response},
};
use sqlx::{Row, sqlite::SqliteRow};
use std::collections::HashMap;

use super::{
    App, CardQuery, Principal, Result, card_page::render_template, cards_query, db_err, escape,
    page_window,
};

const TEMPLATE: &str = include_str!("../static/board.html");

fn filter_form(account: i64, board_id: &str, query: &CardQuery) -> String {
    let term = query.terms.first().map(String::as_str).unwrap_or("");
    let mut preserved = String::new();
    for (key, values) in [
        ("terms[]", query.terms.iter().skip(1).collect::<Vec<_>>()),
        ("card_ids[]", query.card_ids.iter().collect()),
        ("column_ids[]", query.column_ids.iter().collect()),
        ("tag_ids[]", query.tag_ids.iter().collect()),
        ("assignee_ids[]", query.assignee_ids.iter().collect()),
        ("creator_ids[]", query.creator_ids.iter().collect()),
        ("closer_ids[]", query.closer_ids.iter().collect()),
    ] {
        for value in values {
            preserved.push_str(&format!(
                "<input type=\"hidden\" name=\"{key}\" value=\"{}\">",
                escape(value)
            ));
        }
    }
    for (key, value) in [
        ("assignment_status", query.assignment_status.as_deref()),
        ("creation", query.creation.as_deref()),
        ("closure", query.closure.as_deref()),
    ] {
        if let Some(value) = value {
            preserved.push_str(&format!(
                "<input type=\"hidden\" name=\"{key}\" value=\"{}\">",
                escape(value)
            ));
        }
    }
    let option = |value: &str, label: &str, current: Option<&str>| {
        format!(
            "<option value=\"{value}\"{}>{label}</option>",
            if current == Some(value) {
                " selected"
            } else {
                ""
            }
        )
    };
    let index = query.indexed_by.as_deref().unwrap_or("all");
    let sort = query.sorted_by.as_deref().unwrap_or("latest");
    format!(
        "<form class=\"board-filter\" method=\"get\" action=\"/{account}/boards/{}\"><label>Filter these cards<input type=\"search\" name=\"terms[]\" value=\"{}\" placeholder=\"Search cards\"></label>{preserved}<label>Show<select name=\"indexed_by\">{}</select></label><label>Sort<select name=\"sorted_by\">{}</select></label><button class=\"button\" type=\"submit\">Filter</button><a href=\"/{account}/boards/{}\">Clear</a></form>",
        escape(board_id),
        escape(term),
        [
            ("all", "Open"),
            ("closed", "Done"),
            ("not_now", "Not Now"),
            ("stalled", "Stalled"),
            ("postponing_soon", "Closing soon"),
            ("golden", "Golden")
        ]
        .iter()
        .map(|(value, label)| option(value, label, Some(index)))
        .collect::<String>(),
        [
            ("latest", "Latest activity"),
            ("newest", "Newest"),
            ("oldest", "Oldest")
        ]
        .iter()
        .map(|(value, label)| option(value, label, Some(sort)))
        .collect::<String>(),
        escape(board_id)
    )
}

pub(super) async fn show_filtered(
    app: &App,
    principal: &Principal,
    board: &SqliteRow,
    query: &CardQuery,
    raw_query: Option<&str>,
) -> Result<Html<String>> {
    let board_id: String = board.get("id");
    let board_name: String = board.get("name");
    let (offset, limit) = page_window(query.page);
    let mut count = cards_query("SELECT COUNT(*)", principal, query);
    let total: i64 = count
        .build_query_scalar()
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let order = match query.sorted_by.as_deref() {
        Some("oldest") => "c.created_at ASC",
        Some("newest") => "c.created_at DESC",
        _ => "c.last_active_at DESC",
    };
    let mut builder = cards_query("SELECT c.*", principal, query);
    builder
        .push(format!(" ORDER BY {order},c.id DESC LIMIT "))
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
    let cards = builder.build().fetch_all(&app.db).await.map_err(db_err)?;
    let mut list = format!(
        "<div class=\"card-columns card-columns--filtered\" aria-label=\"Filtered cards\"><section class=\"cards cards--grid\"><h2>Filtered cards <span class=\"cards__count\">{total}</span></h2><div class=\"cards__list\">"
    );
    for card in &cards {
        let number: i64 = card.get("number");
        let title: String = card.get("title");
        list.push_str(&format!("<a class=\"card{}\" href=\"/{}/cards/{number}\"><span class=\"card__number\">#{number}</span><strong class=\"card__title\">{}</strong></a>",
            if card.get::<i64,_>("golden") != 0 { " card--golden" } else { "" }, principal.slug, escape(&title)));
    }
    if cards.is_empty() {
        list.push_str("<p class=\"cards__empty\">No cards match this filter.</p>");
    }
    list.push_str("</div>");
    let page_url = |page: i64| {
        let mut serializer = url::form_urlencoded::Serializer::new(String::new());
        for (key, value) in url::form_urlencoded::parse(raw_query.unwrap_or("").as_bytes()) {
            if key != "page" {
                serializer.append_pair(&key, &value);
            }
        }
        serializer.append_pair("page", &page.to_string());
        format!(
            "/{}/boards/{}?{}",
            principal.slug,
            escape(&board_id),
            escape(&serializer.finish())
        )
    };
    if query.page > 1 {
        list.push_str(&format!(
            "<a class=\"button button--secondary\" href=\"{}\">Previous page</a>",
            page_url(query.page - 1)
        ));
    }
    if offset + (cards.len() as i64) < total {
        list.push_str(&format!(
            "<a class=\"button button--secondary\" href=\"{}\">Next page</a>",
            page_url(query.page + 1)
        ));
    }
    list.push_str("</section></div>");
    let values = HashMap::from([
        ("ACCOUNT", principal.slug.to_string()),
        ("BOARD_ID", escape(&board_id)),
        ("BOARD_NAME", escape(&board_name)),
        ("HOME_PATH", format!("/{}/", principal.slug)),
        (
            "SETTINGS_LINK",
            format!(
                "<a class=\"button button--secondary\" href=\"/{}/boards/{}/edit\">Settings</a>",
                principal.slug,
                escape(&board_id)
            ),
        ),
        ("ADD_COLUMN", String::new()),
        ("FILTER_FORM", filter_form(principal.slug, &board_id, query)),
        ("COLUMNS", list),
    ]);
    Ok(Html(render_template(TEMPLATE, &values)))
}

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
    let mut column_markup = "<div class=\"card-columns\" aria-label=\"Board columns\">".to_owned();
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
    column_markup.push_str("</div>");
    let values = HashMap::from([
        ("ACCOUNT", account.to_string()),
        ("BOARD_ID", escape(&board_id)),
        ("BOARD_NAME", escape(&board_name)),
        ("HOME_PATH", format!("/{account}/")),
        ("SETTINGS_LINK", format!("<a class=\"button button--secondary\" href=\"/{account}/boards/{}/edit\">Settings</a>",escape(&board_id))),
        ("ADD_COLUMN", "<button class=\"button button--secondary\" id=\"add-column\" type=\"button\">Add column</button>".to_owned()),
        ("FILTER_FORM", filter_form(account, &board_id, &CardQuery::default())),
        ("COLUMNS", column_markup),
    ]);
    Ok(Html(render_template(TEMPLATE, &values)))
}
