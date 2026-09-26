use axum::{
    http::header,
    response::{Html, IntoResponse, Response},
};
use sqlx::{Row, any::AnyRow as DbRow};
use std::collections::HashMap;

use super::{
    App, CardQuery, Principal, Result, card_page::render_template, card_sort_order, cards_page,
    cards_query, db_err, escape, page_window,
};

const TEMPLATE: &str = include_str!("../static/board.html");

fn webhook_link(principal: &Principal, board_id: &str) -> String {
    if principal.role == "owner" || principal.role == "admin" {
        format!(
            "<a class=\"board-webhooks\" href=\"/{}/boards/{}/webhooks\" aria-label=\"Webhooks\" title=\"Webhooks\"><svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M17.2 14.4c.1-.8.2-1.6.2-2.4s-.1-1.6-.2-2.4h4.1q.3 1.2.3 2.4c0 1.2-.1 1.6-.3 2.4m-6.2 6.7c.7-1.3 1.3-2.8 1.7-4.3h3.5c-1.1 2-3 3.5-5.2 4.3m-.3-6.7H9.2c-.1-.8-.2-1.6-.2-2.4s.1-1.6.2-2.4h5.6c.1.8.2 1.6.2 2.4s-.1 1.6-.2 2.4M12 21.6c-1-1.4-1.8-3-2.3-4.8h4.6c-.5 1.7-1.3 3.3-2.3 4.8M7.2 7.2H3.7c1.1-2 3-3.5 5.2-4.3-.7 1.4-1.3 2.8-1.7 4.3m-3.5 9.6h3.5c.4 1.5 1 2.9 1.7 4.3-2.2-.8-4.1-2.3-5.2-4.3m-1-2.4q-.3-1.2-.3-2.4c0-1.2.1-1.6.3-2.4h4.1c-.1.8-.2 1.6-.2 2.4s.1 1.6.2 2.4m5.2-12c1 1.4 1.8 3 2.3 4.8H9.7c.5-1.7 1.3-3.3 2.3-4.8m8.3 4.8h-3.5c-.4-1.5-.9-2.9-1.7-4.3 2.2.8 4.1 2.3 5.2 4.3M12 0C5.4 0 0 5.4 0 12s5.4 12 12 12 12-5.4 12-12S18.6 0 12 0\"/></svg><span class=\"for-screen-reader\">Webhooks</span></a>",
            principal.slug,
            escape(board_id)
        )
    } else {
        String::new()
    }
}

fn column_color_class(color: &str) -> &'static str {
    match color {
        "var(--color-card-1)" => "cards--color-1",
        "var(--color-card-2)" => "cards--color-2",
        "var(--color-card-3)" => "cards--color-3",
        "var(--color-card-4)" => "cards--color-4",
        "var(--color-card-5)" => "cards--color-5",
        "var(--color-card-6)" => "cards--color-6",
        "var(--color-card-7)" => "cards--color-7",
        "var(--color-card-8)" => "cards--color-8",
        _ => "cards--color-default",
    }
}

async fn filter_form(
    app: &App,
    principal: &Principal,
    board_id: &str,
    query: &CardQuery,
) -> Result<String> {
    let account = principal.slug;
    let term = query.terms.first().map(String::as_str).unwrap_or("");
    let mut preserved = String::new();
    for (key, values) in [
        ("terms[]", query.terms.iter().skip(1).collect::<Vec<_>>()),
        ("card_ids[]", query.card_ids.iter().collect()),
        ("column_ids[]", query.column_ids.iter().collect()),
    ] {
        for value in values {
            preserved.push_str(&format!(
                "<input type=\"hidden\" name=\"{key}\" value=\"{}\">",
                escape(value)
            ));
        }
    }
    for (key, value) in [
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
    let tags = sqlx::query("SELECT id,title FROM tags WHERE account_id=? ORDER BY title,id")
        .bind(&principal.account_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let tag_choices = if tags.is_empty() {
        "<p class=\"filter-choices__empty\">No tags yet</p>".to_owned()
    } else {
        tags.iter()
            .map(|tag| {
                let id: String = tag.get("id");
                let title: String = tag.get("title");
                format!(
                    "<label class=\"filter-choice\"><input type=\"checkbox\" name=\"tag_ids[]\" value=\"{}\"{}><span>#{}</span></label>",
                    escape(&id),
                    if query.tag_ids.contains(&id) { " checked" } else { "" },
                    escape(&title)
                )
            })
            .collect::<String>()
    };
    let users =
        sqlx::query("SELECT id,name FROM users WHERE account_id=? AND active=1 ORDER BY name,id")
            .bind(&principal.account_id)
            .fetch_all(&app.db)
            .await
            .map_err(db_err)?;
    let user_choices = |name: &str, selected: &[String]| {
        users.iter()
            .map(|user| {
                let id: String = user.get("id");
                let display_name: String = user.get("name");
                format!(
                    "<label class=\"filter-choice\"><input type=\"checkbox\" name=\"{name}\" value=\"{}\"{}><span>{}</span></label>",
                    escape(&id),
                    if selected.contains(&id) { " checked" } else { "" },
                    escape(&display_name)
                )
            })
            .collect::<String>()
    };
    let assignee_choices = format!(
        "<label class=\"filter-choice\"><input type=\"checkbox\" name=\"assignment_status\" value=\"unassigned\"{}><span>No one</span></label>{}",
        if query.assignment_status.as_deref() == Some("unassigned") {
            " checked"
        } else {
            ""
        },
        user_choices("assignee_ids[]", &query.assignee_ids)
    );
    let creator_choices = user_choices("creator_ids[]", &query.creator_ids);
    let closer_choices = user_choices("closer_ids[]", &query.closer_ids);
    let clear = if !query.terms.is_empty()
        || !matches!(index, "all")
        || !matches!(sort, "latest")
        || !query.tag_ids.is_empty()
        || !query.assignee_ids.is_empty()
        || !query.creator_ids.is_empty()
        || !query.closer_ids.is_empty()
        || query.assignment_status.is_some()
        || query.creation.is_some()
        || query.closure.is_some()
    {
        format!(
            "<a class=\"filter-clear\" href=\"/{account}/boards/{}\" aria-label=\"Clear filters\" title=\"Clear filters\">×</a>",
            escape(board_id)
        )
    } else {
        String::new()
    };
    Ok(format!(
        "<form class=\"board-filter\" method=\"get\" action=\"/{account}/boards/{}\"><input type=\"hidden\" name=\"board_ids[]\" value=\"{}\"><label class=\"filter-search\"><span class=\"for-screen-reader\">Filter these cards</span><input type=\"search\" name=\"terms[]\" value=\"{}\" placeholder=\"Filter these cards… [F]\" autocomplete=\"off\"></label>{preserved}<details class=\"filter-more\"><summary aria-label=\"Expand filter options\" title=\"Expand filter options\">☷{}</summary><div class=\"filter-more__fields\"><label>Show<select name=\"indexed_by\">{}</select></label><label>Sort<select name=\"sorted_by\">{}</select></label><fieldset class=\"filter-choices\"><legend>Tagged</legend>{tag_choices}</fieldset><fieldset class=\"filter-choices\"><legend>Assigned to</legend>{assignee_choices}</fieldset><fieldset class=\"filter-choices\"><legend>Added by</legend>{creator_choices}</fieldset><fieldset class=\"filter-choices\"><legend>Closed by</legend>{closer_choices}</fieldset><button class=\"button\" type=\"submit\">Filter</button><div id=\"filter-settings-save-toggle\"><button class=\"filter-save\" type=\"button\" aria-label=\"Save custom view\">Save custom view</button><span class=\"filter-save-status\" role=\"status\"></span></div></div></details>{clear}</form>",
        escape(board_id),
        escape(board_id),
        escape(term),
        if query.tag_ids.is_empty()
            && query.assignee_ids.is_empty()
            && query.creator_ids.is_empty()
            && query.closer_ids.is_empty()
            && query.assignment_status.is_none()
        {
            String::new()
        } else {
            format!(
                "<span class=\"filter-count\">{}</span>",
                query.tag_ids.len()
                    + query.assignee_ids.len()
                    + query.creator_ids.len()
                    + query.closer_ids.len()
                    + usize::from(query.assignment_status.is_some())
            )
        },
        [
            ("all", "Open"),
            ("closed", "Done"),
            ("not_now", "Not now"),
            ("stalled", "Stalled"),
            ("postponing_soon", "Closing soon"),
            ("golden", "Golden")
        ]
        .iter()
        .map(|(value, label)| option(value, label, Some(index)))
        .collect::<String>(),
        [
            ("newest", "Newest"),
            ("oldest", "Oldest"),
            ("latest", "Latest activity")
        ]
        .iter()
        .map(|(value, label)| option(value, label, Some(sort)))
        .collect::<String>(),
    ))
}

pub(super) async fn show_filtered(
    app: &App,
    principal: &Principal,
    board: &DbRow,
    query: &CardQuery,
    raw_query: Option<&str>,
    timezone: chrono_tz::Tz,
) -> Result<Html<String>> {
    let board_id: String = board.get("id");
    let board_name: String = board.get("name");
    let (offset, limit) = page_window(query.page);
    let mut count = cards_query("SELECT COUNT(*)", principal, query, app.mysql, timezone);
    let total: i64 = count
        .build_query_scalar()
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let order = card_sort_order(query);
    let mut builder = cards_query(
        "SELECT c.*,b.name AS board_name,(SELECT name FROM columns WHERE id=c.column_id) AS column_name,(SELECT color FROM columns WHERE id=c.column_id) AS column_color,(SELECT name FROM users WHERE id=c.creator_id) AS creator_name",
        principal,
        query,
        app.mysql,
        timezone,
    );
    builder
        .push(format!(" ORDER BY {order} LIMIT "))
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);
    let cards = builder.build().fetch_all(&app.db).await.map_err(db_err)?;
    let mut list = format!(
        "<div class=\"card-columns card-columns--filtered\" aria-label=\"Filtered cards\"><section class=\"cards cards--grid\"><h2>Filtered cards <span class=\"cards__count\">{total}</span></h2><div class=\"cards__list\">"
    );
    list.push_str(&cards_page::previews(app, principal, &cards).await?);
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
        ("WEBHOOKS_LINK", webhook_link(principal, &board_id)),
        (
            "SETTINGS_LINK",
            format!(
                "<a class=\"board-settings\" href=\"/{}/boards/{}/edit\" aria-label=\"Settings for {}\" title=\"Settings for {}\">⚙<span class=\"for-screen-reader\">Settings</span></a>",
                principal.slug,
                escape(&board_id),
                escape(&board_name),
                escape(&board_name)
            ),
        ),
        ("ADD_COLUMN", String::new()),
        (
            "FILTER_FORM",
            filter_form(app, principal, &board_id, query).await?,
        ),
        ("COLUMNS", list),
    ]);
    Ok(Html(render_template(TEMPLATE, &values)))
}

fn column_html(
    account: i64,
    board_id: &str,
    id: &str,
    title: &str,
    color: &str,
    count: i64,
    cards: Option<&Vec<DbRow>>,
    rendered_cards: Option<&str>,
    path: &str,
    add_card: bool,
    board_footer: Option<&str>,
) -> String {
    let dom_id = match id {
        "done" => "closed-cards".to_owned(),
        "maybe" | "not-now" => id.to_owned(),
        _ => format!("column_{id}"),
    };
    let mut html = format!(
        "<section class=\"cards cards--{} {} {}\" id=\"{}\" data-column-id=\"{}\" data-count=\"{count}\" aria-label=\"{}\"><header class=\"cards__header\"><button class=\"cards__toggle\" type=\"button\" aria-expanded=\"{}\"><span class=\"cards__count\">{count}</span><h2>{}</h2></button></header><div class=\"cards__list\">",
        if id == "maybe" {
            "maybe"
        } else if id == "done" {
            "closed"
        } else if id == "not-now" {
            "on-deck"
        } else {
            "doing"
        },
        if id == "maybe" {
            "is-expanded"
        } else {
            "is-collapsed"
        },
        column_color_class(color),
        escape(&dom_id),
        escape(id),
        escape(title),
        if id == "maybe" { "true" } else { "false" },
        escape(title),
    );
    if add_card {
        html.push_str(&format!(
            "<div class=\"board-tools\"><form id=\"add-card\" action=\"/{account}/boards/{}/cards\" method=\"post\"><button class=\"button\" type=\"submit\">Add a card <kbd>C</kbd></button></form>{}</div>",
            escape(board_id), board_footer.unwrap_or("")
        ));
    }
    if let Some(rendered) = rendered_cards {
        html.push_str(rendered);
    } else if let Some(cards) = cards {
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

pub(super) fn named_column_html(
    account: i64,
    board_id: &str,
    id: &str,
    name: &str,
    color: &str,
    count: i64,
) -> String {
    column_html(
        account,
        board_id,
        id,
        name,
        color,
        count,
        None,
        None,
        &format!("/columns/{id}"),
        false,
        None,
    )
}

async fn board_footer(app: &App, principal: &Principal, board_id: &str) -> Result<String> {
    let rows = sqlx::query("SELECT u.id,u.name FROM accesses a JOIN users u ON u.id=a.user_id WHERE a.board_id=? AND a.involvement='watching' AND u.active=1 ORDER BY lower(u.name),u.id")
        .bind(board_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let watching = rows
        .iter()
        .any(|row| row.get::<String, _>("id") == principal.user_id);
    let heading = if rows.is_empty() {
        "No one is watching for new cards"
    } else {
        "Watching for new cards"
    };
    let mut avatars = String::new();
    for row in rows.iter().take(8) {
        let name: String = row.get("name");
        let initial = name
            .chars()
            .next()
            .unwrap_or('?')
            .to_uppercase()
            .collect::<String>();
        avatars.push_str(&format!(
            "<span class=\"board-tools__avatar\" title=\"{}\" aria-label=\"{}\">{}</span>",
            escape(&name),
            escape(&name),
            escape(&initial)
        ));
    }
    if rows.len() > 8 {
        avatars.push_str(&format!("<span class=\"board-tools__avatar board-tools__overflow\" aria-label=\"{} more watchers\">+{}</span>",rows.len()-8,rows.len()-8));
    }
    Ok(format!(
        "<footer class=\"board-tools__footer\"><div class=\"board-tools__divider\"><strong>{heading}</strong></div><div class=\"board-tools__watching\">{avatars}</div><button class=\"board-tools__watch-toggle\" type=\"button\" data-involvement=\"{}\"><svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M18 8a6 6 0 0 0-12 0c0 7-3 8-3 9h18c0-1-3-2-3-9M10 21h4\"/></svg>{}</button></footer>",
        if watching { "access_only" } else { "watching" },
        if watching {
            "Stop watching"
        } else {
            "Watch this"
        }
    ))
}

pub(super) async fn drop_column_response(
    app: &App,
    principal: &Principal,
    board_id: &str,
    destination: &str,
    column_id: Option<&str>,
) -> Result<Response> {
    let (id, title, color, path, add_card) = match destination {
        "closure" => (
            "done".to_owned(),
            "Done".to_owned(),
            "var(--color-card-complete)".to_owned(),
            "/columns/closed".to_owned(),
            false,
        ),
        "not_now" => (
            "not-now".to_owned(),
            "Not Now".to_owned(),
            "var(--color-card-complete)".to_owned(),
            "/columns/not_now".to_owned(),
            false,
        ),
        "stream" => (
            "maybe".to_owned(),
            "Maybe?".to_owned(),
            "var(--color-card-default)".to_owned(),
            "/columns/stream".to_owned(),
            true,
        ),
        "column" => {
            let id = column_id.ok_or_else(|| super::bad("column_id is required"))?;
            let row = super::checked_column(app, principal, board_id, id).await?;
            (
                id.to_owned(),
                row.get("name"),
                row.get("color"),
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
    let footer = if add_card {
        Some(board_footer(app, principal, board_id).await?)
    } else {
        None
    };
    let content = column_html(
        principal.slug,
        board_id,
        &id,
        &title,
        &color,
        count,
        Some(&cards),
        None,
        &path,
        add_card,
        footer.as_deref(),
    );
    Ok(([(header::CONTENT_TYPE, "text/vnd.turbo-stream.html; charset=utf-8")],
        format!("<turbo-stream action=\"replace\" method=\"morph\" target=\"{}\"><template>{content}</template></turbo-stream>", escape(&target))).into_response())
}

pub(super) async fn show(
    app: &App,
    principal: &Principal,
    board: &DbRow,
    page: i64,
) -> Result<Html<String>> {
    let board_id: String = board.get("id");
    let board_name: String = board.get("name");
    let columns =
        sqlx::query("SELECT id,name,color FROM columns WHERE board_id=? ORDER BY position,id")
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
    let (offset, limit) = page_window(page);
    let maybe = sqlx::query("SELECT c.*,b.name AS board_name,(SELECT name FROM columns WHERE id=c.column_id) AS column_name,(SELECT color FROM columns WHERE id=c.column_id) AS column_color,(SELECT name FROM users WHERE id=c.creator_id) AS creator_name FROM cards c JOIN boards b ON b.id=c.board_id WHERE c.board_id=? AND c.column_id IS NULL AND c.status='published' AND c.closed=0 AND c.postponed=0 ORDER BY c.golden DESC,c.last_active_at DESC,c.id DESC LIMIT ? OFFSET ?")
        .bind(&board_id)
        .bind(limit)
        .bind(offset)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let maybe_html = cards_page::previews(app, principal, &maybe).await?;
    let footer = board_footer(app, principal, &board_id).await?;
    let account = principal.slug;
    let mut column_markup = "<div class=\"card-columns\" aria-label=\"Board columns\">".to_owned();
    column_markup.push_str(&column_html(
        account,
        &board_id,
        "not-now",
        "Not Now",
        "var(--color-card-complete)",
        *counts.get("not-now").unwrap_or(&0),
        None,
        None,
        "/columns/not_now",
        false,
        None,
    ));
    column_markup.push_str(&column_html(
        account,
        &board_id,
        "maybe",
        "Maybe?",
        "var(--color-card-default)",
        *counts.get("maybe").unwrap_or(&0),
        Some(&maybe),
        Some(&maybe_html),
        "/columns/stream",
        true,
        Some(&footer),
    ));
    for column in columns {
        let id: String = column.get("id");
        let name: String = column.get("name");
        let color: String = column.get("color");
        column_markup.push_str(&column_html(
            account,
            &board_id,
            &id,
            &name,
            &color,
            *counts.get(&id).unwrap_or(&0),
            None,
            None,
            &format!("/columns/{id}/cards"),
            false,
            None,
        ));
    }
    column_markup.push_str(&column_html(
        account,
        &board_id,
        "done",
        "Done",
        "var(--color-card-complete)",
        *counts.get("done").unwrap_or(&0),
        None,
        None,
        "/columns/closed",
        false,
        None,
    ));
    column_markup.push_str("</div>");
    let values = HashMap::from([
        ("ACCOUNT", account.to_string()),
        ("BOARD_ID", escape(&board_id)),
        ("BOARD_NAME", escape(&board_name)),
        ("HOME_PATH", format!("/{account}/")),
        ("WEBHOOKS_LINK", webhook_link(principal, &board_id)),
        ("SETTINGS_LINK", format!("<a class=\"board-settings\" href=\"/{account}/boards/{}/edit\" aria-label=\"Settings for {}\" title=\"Settings for {}\">⚙<span class=\"for-screen-reader\">Settings</span></a>",escape(&board_id),escape(&board_name),escape(&board_name))),
        ("ADD_COLUMN", "<button class=\"board-add-column\" id=\"add-column\" type=\"button\" aria-label=\"Add a column\" title=\"Add a column\">+<span class=\"for-screen-reader\">Add column</span></button>".to_owned()),
        ("FILTER_FORM", filter_form(app, principal, &board_id, &CardQuery::default()).await?),
        ("COLUMNS", column_markup),
    ]);
    Ok(Html(render_template(TEMPLATE, &values)))
}
