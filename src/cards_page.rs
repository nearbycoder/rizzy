use axum::response::Html;
use sqlx::{Row, sqlite::SqliteRow};
use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
};

use super::{
    App, CardQuery, Principal, Result, card_page::render_template, db_err, escape, page_window,
};

const TEMPLATE: &str = include_str!("../static/cards.html");

pub(super) fn previews(principal: &Principal, cards: &[SqliteRow]) -> String {
    let mut html = String::new();
    for card in cards {
        let id: String = card.get("id");
        let number: i64 = card.get("number");
        let title: String = card.get("title");
        let board_name: String = card.get("board_name");
        let closed = card.get::<i64, _>("closed") != 0;
        let postponed = card.get::<i64, _>("postponed") != 0;
        let golden = card.get::<i64, _>("golden") != 0;
        let status = if closed {
            "Done"
        } else if postponed {
            "Not now"
        } else {
            "Open"
        };
        let class = if golden { " card--golden" } else { "" };
        let _ = write!(
            html,
            "<article id=\"article_card_{}\" class=\"card{}\" data-id=\"{number}\" tabindex=\"0\"><a class=\"card__link\" href=\"/{}/cards/{number}\" title=\"{} in {}\"><span class=\"card__board\">{}</span><span class=\"card__number\">#{number}</span><h3 class=\"card__title\">{}</h3><span class=\"card__status\">{status}</span></a></article>",
            escape(&id),
            class,
            principal.slug,
            escape(&title),
            escape(&board_name),
            escape(&board_name),
            escape(&title)
        );
    }
    html
}

fn page_query(raw: Option<&str>, page: i64, target: Option<&str>) -> String {
    let mut params = url::form_urlencoded::Serializer::new(String::new());
    for (key, value) in url::form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
        if key != "page" && key != "target" {
            params.append_pair(&key, &value);
        }
    }
    if let Some(target) = target {
        params.append_pair("target", target);
    }
    params.append_pair("page", &page.to_string());
    params.finish()
}

pub(super) fn next_link(
    principal: &Principal,
    target: &str,
    page: i64,
    raw: Option<&str>,
) -> String {
    let page_url = format!("/{}/cards?{}", principal.slug, page_query(raw, page, None));
    let preview_url = format!(
        "/{}/cards/previews.turbo_stream?{}",
        principal.slug,
        page_query(raw, page, Some(target))
    );
    format!(
        "<a id=\"{}-load-page-{page}\" class=\"cards__more\" href=\"{}\" data-preview-url=\"{}\">Load more cards</a>",
        escape(target),
        escape(&page_url),
        escape(&preview_url)
    )
}

fn option(value: &str, label: &str, current: Option<&str>) -> String {
    format!(
        "<option value=\"{}\"{}>{}</option>",
        escape(value),
        if current == Some(value) {
            " selected"
        } else {
            ""
        },
        escape(label)
    )
}

fn choice_group(
    label: &str,
    name: &str,
    choices: &[(String, String)],
    selected: &[String],
) -> String {
    let selected_ids: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let selected_names: Vec<&str> = choices
        .iter()
        .filter(|(id, _)| selected_ids.contains(id.as_str()))
        .map(|(_, title)| title.as_str())
        .collect();
    let summary = match selected_names.len() {
        0 => label.to_owned(),
        1 => selected_names[0].to_owned(),
        amount => format!("{label} ({amount})"),
    };
    let mut html = format!(
        "<details class=\"choice-popover\"><summary>{}</summary><div class=\"choice-popover__items\" role=\"group\" aria-label=\"{}\">",
        escape(&summary),
        escape(label)
    );
    for (id, title) in choices {
        let checked = if selected_ids.contains(id.as_str()) {
            " checked"
        } else {
            ""
        };
        let _ = write!(
            html,
            "<label class=\"choice-popover__item\"><input type=\"checkbox\" name=\"{}\" value=\"{}\"{checked}><span>{}</span></label>",
            escape(name),
            escape(id),
            escape(title)
        );
    }
    html.push_str("</div></details>");
    html
}

fn board_label(boards: &[SqliteRow], selected: &[String]) -> String {
    let selected: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let names: Vec<String> = if selected.is_empty() {
        if boards.len() == 1 {
            vec![boards[0].get("name")]
        } else {
            vec!["All boards".into()]
        }
    } else {
        boards
            .iter()
            .filter(|board| selected.contains(board.get::<String, _>("id").as_str()))
            .map(|board| board.get("name"))
            .collect()
    };
    match names.len() {
        0 => "All boards".into(),
        1 => names[0].clone(),
        2 => format!("{} and {}", names[0], names[1]),
        _ => format!(
            "{}, and {}",
            names[..names.len() - 1].join(", "),
            names[names.len() - 1]
        ),
    }
}

pub(super) async fn show(
    app: &App,
    principal: &Principal,
    cards: &[SqliteRow],
    total: i64,
    query: &CardQuery,
    raw: Option<&str>,
) -> Result<Html<String>> {
    let boards = sqlx::query("SELECT b.id,b.name FROM boards b JOIN accesses x ON x.board_id=b.id WHERE b.account_id=? AND x.user_id=? ORDER BY lower(b.name),b.id")
        .bind(&principal.account_id).bind(&principal.user_id).fetch_all(&app.db).await.map_err(db_err)?;
    let tags = sqlx::query("SELECT id,title FROM tags WHERE account_id=? ORDER BY lower(title),id")
        .bind(&principal.account_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let users = sqlx::query("SELECT id,name FROM users WHERE account_id=? AND active=1 AND role!='system' ORDER BY lower(name),id")
        .bind(&principal.account_id).fetch_all(&app.db).await.map_err(db_err)?;
    let title = board_label(&boards, &query.board_ids);
    let board_choices: Vec<(String, String)> = boards
        .iter()
        .map(|board| (board.get("id"), board.get("name")))
        .collect();
    let tag_choices: Vec<(String, String)> = tags
        .iter()
        .map(|tag| (tag.get("id"), format!("#{}", tag.get::<String, _>("title"))))
        .collect();
    let user_choices: Vec<(String, String)> = users
        .iter()
        .map(|user| (user.get("id"), user.get("name")))
        .collect();
    let board_control = choice_group("Board", "board_ids[]", &board_choices, &query.board_ids);
    let tag_control = choice_group("Tag", "tag_ids[]", &tag_choices, &query.tag_ids);
    let assignee_control = choice_group(
        "Assigned to",
        "assignee_ids[]",
        &user_choices,
        &query.assignee_ids,
    );
    let creator_control = choice_group(
        "Added by",
        "creator_ids[]",
        &user_choices,
        &query.creator_ids,
    );
    let closer_control = choice_group(
        "Closed by",
        "closer_ids[]",
        &user_choices,
        &query.closer_ids,
    );
    let mut preserved = String::new();
    for (key, values) in [
        ("terms[]", query.terms.iter().skip(1).collect::<Vec<_>>()),
        ("column_ids[]", query.column_ids.iter().collect()),
        ("card_ids[]", query.card_ids.iter().collect()),
    ] {
        for value in values {
            let _ = write!(
                preserved,
                "<input type=\"hidden\" name=\"{key}\" value=\"{}\">",
                escape(value)
            );
        }
    }
    let statuses = [
        ("all", "Open"),
        ("maybe", "Maybe?"),
        ("not_now", "Not now"),
        ("closed", "Done"),
        ("golden", "Golden"),
        ("stalled", "Stalled"),
        ("postponing_soon", "Closing soon"),
    ];
    let status_options = statuses
        .iter()
        .map(|(value, label)| option(value, label, query.indexed_by.as_deref()))
        .collect::<String>();
    let sort_options = [
        ("latest", "Latest activity"),
        ("newest", "Newest"),
        ("oldest", "Oldest"),
    ]
    .iter()
    .map(|(value, label)| option(value, label, query.sorted_by.as_deref()))
    .collect::<String>();
    let assignment_options = [("", "Any assignment"), ("unassigned", "Unassigned")]
        .iter()
        .map(|(value, label)| option(value, label, query.assignment_status.as_deref()))
        .collect::<String>();
    let mut list = previews(principal, cards);
    if cards.is_empty() {
        list.push_str("<div class=\"blank-slate\">No cards match this filter</div>");
    }
    let (offset, _) = page_window(query.page);
    if offset + (cards.len() as i64) < total {
        list.push_str(&next_link(
            principal,
            "cards_paginated_container",
            query.page + 1,
            raw,
        ));
    }
    let previous = if query.page > 1 {
        let url = format!(
            "/{}/cards?{}",
            principal.slug,
            page_query(raw, query.page - 1, None)
        );
        format!(
            "<a class=\"page-link\" href=\"{}\">Previous page</a>",
            escape(&url)
        )
    } else {
        String::new()
    };
    let form = format!(
        "<form class=\"filter-form\" action=\"/{}/cards\" method=\"get\"><label>Filter these cards<input type=\"search\" name=\"terms[]\" value=\"{}\" placeholder=\"Filter these cards… [F]\"></label>{board_control}<label>Status<select name=\"indexed_by\">{status_options}</select></label><label>Sort<select name=\"sorted_by\">{sort_options}</select></label><details class=\"more-filters\"><summary>More filters</summary><div class=\"more-filters__fields\">{tag_control}{assignee_control}{creator_control}{closer_control}<label>Assignment<select name=\"assignment_status\">{assignment_options}</select></label><label>Created<input name=\"creation\" value=\"{}\" placeholder=\"This week\"></label><label>Closed<input name=\"closure\" value=\"{}\" placeholder=\"This month\"></label></div></details>{preserved}<button type=\"submit\">Apply filters</button><a class=\"clear-link\" href=\"/{}/cards\">Clear all</a></form>",
        principal.slug,
        escape(query.terms.first().map(String::as_str).unwrap_or("")),
        escape(query.creation.as_deref().unwrap_or("")),
        escape(query.closure.as_deref().unwrap_or("")),
        principal.slug
    );
    let values = HashMap::from([
        ("ACCOUNT", principal.slug.to_string()),
        ("TITLE", escape(&title)),
        ("FILTER_FORM", form),
        ("CARDS", list),
        ("COUNT", total.to_string()),
        ("PREVIOUS", previous),
    ]);
    Ok(Html(render_template(TEMPLATE, &values)))
}
