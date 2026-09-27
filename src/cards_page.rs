use axum::response::Html;
use sqlx::{Any as Db, QueryBuilder, Row, any::AnyRow as DbRow};
use std::{
    collections::{HashMap, HashSet},
    fmt::Write,
};

use super::{
    App, CardQuery, Principal, Result, card_page::render_template, db_err, escape, page_window,
    storage,
};

const TEMPLATE: &str = include_str!("../static/cards.html");

fn author_label(name: &str) -> (String, String) {
    let words: Vec<&str> = name.split_whitespace().collect();
    let initials = words
        .iter()
        .take(2)
        .filter_map(|word| word.chars().next())
        .collect::<String>()
        .to_uppercase();
    let familiar = match words.as_slice() {
        [] => String::new(),
        [first] => (*first).to_owned(),
        [first, .., last] => format!("{} {}.", first, last.chars().next().unwrap_or_default()),
    };
    (initials, familiar)
}

#[derive(Default)]
struct PreviewMeta {
    tags: Vec<String>,
    steps: i64,
    completed_steps: i64,
    comments: i64,
    boosts: i64,
    assignees: Vec<(String, String)>,
}

async fn preview_metadata(app: &App, cards: &[DbRow]) -> Result<HashMap<String, PreviewMeta>> {
    let mut metadata = HashMap::new();
    if cards.is_empty() {
        return Ok(metadata);
    }
    let ids: Vec<String> = cards.iter().map(|card| card.get("id")).collect();
    let step_relation = if app.mysql {
        "SELECT 'step' AS kind,s.card_id,NULL AS label,NULL AS user_id,CAST(COALESCE(SUM(s.completed),0) AS SIGNED) AS completed,COUNT(*) AS amount FROM steps s WHERE s.card_id IN ("
    } else {
        "SELECT 'step' AS kind,s.card_id,NULL AS label,NULL AS user_id,COALESCE(SUM(s.completed),0) AS completed,COUNT(*) AS amount FROM steps s WHERE s.card_id IN ("
    };
    // Each relationship is read in one page-scoped query. The UNION avoids a
    // per-card set of round trips while leaving the JSON card API untouched.
    let relations = [
        (
            "SELECT 'tag' AS kind,g.card_id,t.title AS label,NULL AS user_id,0 AS completed,1 AS amount FROM taggings g JOIN tags t ON t.id=g.tag_id WHERE g.card_id IN (",
            "",
        ),
        (step_relation, " GROUP BY s.card_id"),
        (
            "SELECT 'comment' AS kind,m.card_id,NULL AS label,NULL AS user_id,0 AS completed,COUNT(*) AS amount FROM comments m JOIN users u ON u.id=m.creator_id AND u.role!='system' WHERE m.card_id IN (",
            " GROUP BY m.card_id",
        ),
        (
            "SELECT 'boost' AS kind,r.reactable_id AS card_id,NULL AS label,NULL AS user_id,0 AS completed,COUNT(*) AS amount FROM reactions r WHERE r.reactable_type='Card' AND r.reactable_id IN (",
            " GROUP BY r.reactable_id",
        ),
        (
            "SELECT 'assignee' AS kind,a.card_id,u.name AS label,u.id AS user_id,0 AS completed,1 AS amount FROM assignments a JOIN users u ON u.id=a.assignee_id WHERE a.card_id IN (",
            "",
        ),
    ];
    let mut builder = QueryBuilder::<Db>::new("");
    for (index, (relation, suffix)) in relations.iter().enumerate() {
        if index > 0 {
            builder.push(" UNION ALL ");
        }
        builder.push(*relation);
        let mut separated = builder.separated(",");
        for id in &ids {
            separated.push_bind(id);
        }
        separated.push_unseparated(")");
        builder.push(*suffix);
    }
    builder.push(" ORDER BY kind,label");
    for row in builder.build().fetch_all(&app.db).await.map_err(db_err)? {
        let id: String = row.get("card_id");
        let item = metadata.entry(id).or_insert_with(PreviewMeta::default);
        match row.get::<String, _>("kind").as_str() {
            "tag" => item.tags.push(row.get("label")),
            "step" => {
                item.steps += row.get::<i64, _>("amount");
                item.completed_steps += row.get::<i64, _>("completed");
            }
            "comment" => item.comments += row.get::<i64, _>("amount"),
            "boost" => item.boosts += row.get::<i64, _>("amount"),
            "assignee" => item.assignees.push((row.get("user_id"), row.get("label"))),
            _ => {}
        }
    }
    Ok(metadata)
}

pub(super) async fn previews(app: &App, principal: &Principal, cards: &[DbRow]) -> Result<String> {
    Ok(preview_fragments(app, principal, cards).await?.concat())
}

pub(super) async fn preview_fragments(
    app: &App,
    principal: &Principal,
    cards: &[DbRow],
) -> Result<Vec<String>> {
    let metadata = preview_metadata(app, cards).await?;
    let ids: Vec<String> = cards.iter().map(|card| card.get("id")).collect();
    let images = storage::image_urls_for_cards(app, principal, &ids).await?;
    let mut fragments = Vec::with_capacity(cards.len());
    for card in cards {
        let mut html = String::new();
        let id: String = card.get("id");
        let meta = metadata.get(&id);
        let number: i64 = card.get("number");
        let title: String = card.get("title");
        let board_name: String = card.get("board_name");
        let creator_name: Option<String> = card.get("creator_name");
        let (_, familiar_name) = author_label(creator_name.as_deref().unwrap_or(""));
        let creator_id: String = card.get("creator_id");
        let creator_avatar = format!(
            "<img class=\"card__avatar\" src=\"/{}/users/{}/avatar\" alt=\"\" title=\"{}\" width=\"30\" height=\"30\">",
            principal.slug,
            escape(&creator_id),
            escape(creator_name.as_deref().unwrap_or(""))
        );
        let column_name: Option<String> = card.get("column_name");
        let column_color: Option<String> = card.get("column_color");
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
        let color = match column_color.as_deref() {
            Some("var(--color-card-1)") => "card--color-1",
            Some("var(--color-card-2)") => "card--color-2",
            Some("var(--color-card-3)") => "card--color-3",
            Some("var(--color-card-4)") => "card--color-4",
            Some("var(--color-card-5)") => "card--color-5",
            Some("var(--color-card-6)") => "card--color-6",
            Some("var(--color-card-7)") => "card--color-7",
            Some("var(--color-card-8)") => "card--color-8",
            _ => "card--color-default",
        };
        let column = column_name
            .as_ref()
            .map(|name| format!("<span class=\"card__column-name\">{}</span>", escape(name)))
            .unwrap_or_default();
        let tags = meta
            .filter(|item| !item.tags.is_empty())
            .map(|item| {
                let list = item
                    .tags
                    .iter()
                    .map(|tag| format!("<span class=\"card__tag\">{}</span>", escape(tag)))
                    .collect::<String>();
                format!("<div class=\"card__tags\" aria-label=\"Tags\">{list}</div>")
            })
            .unwrap_or_default();
        let steps = meta
            .filter(|item| item.steps > 0)
            .map(|item| {
                format!(
                    "<div class=\"card__steps\" aria-label=\"Steps completed\">{}/{}</div>",
                    item.completed_steps, item.steps
                )
            })
            .unwrap_or_default();
        let assignees = meta
            .filter(|item| !item.assignees.is_empty())
            .map(|item| {
                let names = item.assignees.iter().map(|(_, name)| name.as_str()).collect::<Vec<_>>().join(" / ");
                format!("<span class=\"card__meta-text card__meta-text--assignees\">Assigned to {}</span>", escape(&names))
            })
            .unwrap_or_default();
        let assignee_avatars = meta
            .map(|item| item.assignees.iter().map(|(user_id, name)| format!(
                "<img class=\"card__assignee-avatar\" src=\"/{}/users/{}/avatar\" alt=\"\" title=\"{}\" width=\"24\" height=\"24\">",
                principal.slug, escape(user_id), escape(name)
            )).collect::<String>())
            .unwrap_or_default();
        let background = images
            .get(&id)
            .and_then(|value| value.as_str())
            .map(|url| format!("<div class=\"card__background\"><img src=\"{}\" alt=\"\" loading=\"lazy\"></div>", escape(url)))
            .unwrap_or_default();
        let counts = meta
            .map(|item| {
                let mut output = String::new();
                if item.boosts > 0 {
                    let _ = write!(
                        output,
                        "<span class=\"card__boosts\" aria-label=\"Boosts\">▲ {}</span>",
                        item.boosts
                    );
                }
                if item.comments > 0 {
                    let _ = write!(
                        output,
                        "<span class=\"card__comments\" aria-label=\"Comments\">◌ {}</span>",
                        item.comments
                    );
                }
                output
            })
            .unwrap_or_default();
        let created_at: String = card.get("created_at");
        let last_active_at: String = card.get("last_active_at");
        let stamp = if closed || postponed {
            format!(
                "<div class=\"card__closed\"><span class=\"card__closed-title\">{status}</span></div>"
            )
        } else {
            String::new()
        };
        let _ = write!(
            html,
            "<article id=\"article_card_{}\" class=\"card{} {}\" data-id=\"{number}\" data-card-number=\"{number}\" draggable=\"true\" tabindex=\"0\"><div class=\"card__main\"><a class=\"card__link\" href=\"/{}/cards/{number}\" title=\"{} in {}\"><span class=\"for-screen-reader\">{}</span></a><header class=\"card__header\"><div class=\"card__board\"><div class=\"card__id\"><span class=\"for-screen-reader\">Card number</span>{number}</div><div class=\"card__board-name\"><span class=\"overflow-ellipsis\">{}</span></div></div>{tags}{steps}{column}</header><div class=\"card__body\"><div class=\"card__content\"><h3 class=\"card__title overflow-line-clamp\">{}</h3></div>{stamp}</div></div><footer class=\"card__footer\"><div class=\"card__meta\">{creator_avatar}<span class=\"card__meta-text card__meta-text--added\">Added <time datetime=\"{}\" data-relative-date>{}</time></span><span class=\"card__meta-text card__meta-text--updated\">↻ <time datetime=\"{}\" data-relative-date>{}</time></span><span class=\"card__meta-text card__meta-text--author\">{}</span>{assignees}<div class=\"card__assignee-avatars\">{assignee_avatars}</div></div><div class=\"card__counts\">{counts}</div>{background}</footer></article>",
            escape(&id),
            class,
            color,
            principal.slug,
            escape(&title),
            escape(&board_name),
            escape(&title),
            escape(&board_name),
            escape(&title),
            escape(&created_at),
            escape(created_at.get(..10).unwrap_or(&created_at)),
            escape(&last_active_at),
            escape(last_active_at.get(..10).unwrap_or(&last_active_at)),
            escape(&familiar_name)
        );
        fragments.push(html);
    }
    Ok(fragments)
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

fn radio_option(name: &str, value: &str, label: &str, current: &str) -> String {
    format!(
        "<label class=\"quick-filter__choice\"><input type=\"radio\" name=\"{}\" value=\"{}\"{}><span>{}</span></label>",
        escape(name),
        escape(value),
        if current == value { " checked" } else { "" },
        escape(label)
    )
}

fn quick_filter(label: &str, selected: bool, searchable: bool, choices: &str) -> String {
    format!(
        "<div class=\"quick-filter{}\"><button class=\"quick-filter__toggle\" type=\"button\" aria-expanded=\"false\">{}</button><div class=\"quick-filter__popup\" role=\"dialog\" aria-label=\"{}\" tabindex=\"-1\" hidden>{}{choices}</div></div>",
        if selected { " is-selected" } else { "" },
        escape(label),
        escape(label),
        if searchable {
            "<input class=\"quick-filter__search\" type=\"search\" placeholder=\"Filter…\" aria-label=\"Filter…\" autocomplete=\"off\" autocorrect=\"off\">".to_owned()
        } else {
            String::new()
        }
    )
}

fn choice_group(
    label: &str,
    name: &str,
    choices: &[(String, String)],
    selected: &[String],
    extra: Option<(&str, &str, &str, bool)>,
) -> String {
    let selected_ids: HashSet<&str> = selected.iter().map(String::as_str).collect();
    let mut selected_names: Vec<&str> = choices
        .iter()
        .filter(|(id, _)| selected_ids.contains(id.as_str()))
        .map(|(_, title)| title.as_str())
        .collect();
    if let Some((_, _, title, true)) = extra {
        selected_names.push(title);
    }
    let prefix = match name {
        "assignee_ids[]" => "Assigned to ",
        "creator_ids[]" => "Added by ",
        "closer_ids[]" => "Closed by ",
        _ => "",
    };
    let summary = match selected_names.len() {
        0 => label.to_owned(),
        1 => format!("{prefix}{}", selected_names[0]),
        2 => format!("{prefix}{} or {}", selected_names[0], selected_names[1]),
        count => format!(
            "{prefix}{}, or {}",
            selected_names[..count - 1].join(", "),
            selected_names[count - 1]
        ),
    };
    let mut choices_html = String::new();
    if let Some((field, value, title, checked)) = extra {
        let _ = write!(
            choices_html,
            "<label class=\"quick-filter__choice\"><input type=\"checkbox\" name=\"{}\" value=\"{}\"{}><span>{}</span></label>",
            escape(field),
            escape(value),
            if checked { " checked" } else { "" },
            escape(title)
        );
    }
    for (id, title) in choices {
        let checked = if selected_ids.contains(id.as_str()) {
            " checked"
        } else {
            ""
        };
        let _ = write!(
            choices_html,
            "<label class=\"quick-filter__choice\"><input type=\"checkbox\" name=\"{}\" value=\"{}\"{checked}><span>{}</span></label>",
            escape(name),
            escape(id),
            escape(title)
        );
    }
    if choices_html.is_empty() {
        String::new()
    } else {
        quick_filter(
            &summary,
            !selected_names.is_empty(),
            choices.len() > 1,
            &choices_html,
        )
    }
}

fn board_label(boards: &[DbRow], selected: &[String]) -> String {
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
    cards: &[DbRow],
    total: i64,
    query: &CardQuery,
    raw: Option<&str>,
) -> Result<Html<String>> {
    let boards = sqlx::query("SELECT b.id,b.name FROM boards b JOIN accesses x ON x.board_id=b.id WHERE b.account_id=? AND x.user_id=? ORDER BY x.accessed_at DESC,b.id")
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
    let board_control = choice_group(
        "Board…",
        "board_ids[]",
        &board_choices,
        &query.board_ids,
        None,
    );
    let tag_control = choice_group("Tagged…", "tag_ids[]", &tag_choices, &query.tag_ids, None);
    let assignee_control = choice_group(
        "Assigned to…",
        "assignee_ids[]",
        &user_choices,
        &query.assignee_ids,
        Some((
            "assignment_status",
            "unassigned",
            "No one",
            query.assignment_status.as_deref() == Some("unassigned"),
        )),
    );
    let creator_control = choice_group(
        "Added by…",
        "creator_ids[]",
        &user_choices,
        &query.creator_ids,
        None,
    );
    let closer_control = choice_group(
        "Closed by…",
        "closer_ids[]",
        &user_choices,
        &query.closer_ids,
        None,
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
    for (key, value) in [
        ("creation", query.creation.as_deref()),
        ("closure", query.closure.as_deref()),
    ] {
        if let Some(value) = value {
            let _ = write!(
                preserved,
                "<input type=\"hidden\" name=\"{key}\" value=\"{}\">",
                escape(value)
            );
        }
    }
    let expanded = raw.is_some_and(|raw| {
        url::form_urlencoded::parse(raw.as_bytes())
            .any(|(key, value)| key == "expand_all" && value == "true")
    });
    let index = query.indexed_by.as_deref().unwrap_or("all");
    let sort = query.sorted_by.as_deref().unwrap_or("latest");
    let statuses = [
        ("all", "Open"),
        ("closed", "Done"),
        ("not_now", "Not now"),
        ("stalled", "Stalled"),
        ("postponing_soon", "Closing soon"),
        ("golden", "Golden"),
    ];
    let status_options = statuses
        .iter()
        .map(|(value, label)| radio_option("indexed_by", value, label, index))
        .collect::<String>();
    let sort_options = [
        ("newest", "Newest to oldest"),
        ("oldest", "Oldest to newest"),
        ("latest", "Recently updated"),
    ]
    .iter()
    .map(|(value, label)| radio_option("sorted_by", value, label, sort))
    .collect::<String>();
    let status_label = match index {
        "all" => "Status…",
        "closed" => "Done",
        "not_now" => "Not now",
        "stalled" => "Stalled",
        "postponing_soon" => "Closing soon",
        "golden" => "Golden",
        _ => "Status…",
    };
    let sort_label = match sort {
        "newest" => "Newest to oldest",
        "oldest" => "Oldest to newest",
        _ => "Recently updated",
    };
    let controls = [
        board_control,
        quick_filter(sort_label, sort != "latest", false, &sort_options),
        quick_filter(status_label, index != "all", false, &status_options),
        tag_control,
        assignee_control,
        creator_control,
        closer_control,
    ]
    .concat();
    let filters_used = !query.board_ids.is_empty()
        || !query.terms.is_empty()
        || index != "all"
        || sort != "latest"
        || !query.tag_ids.is_empty()
        || !query.assignee_ids.is_empty()
        || !query.creator_ids.is_empty()
        || !query.closer_ids.is_empty()
        || query.assignment_status.is_some()
        || query.creation.is_some()
        || query.closure.is_some();
    let clear = if filters_used {
        format!(
            "<a class=\"clear-link\" href=\"/{}/cards\" aria-label=\"Clear all\" title=\"Clear all\">×</a>",
            principal.slug
        )
    } else {
        String::new()
    };
    let mut list = previews(app, principal, cards).await?;
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
        "<form class=\"filter-form{}\" action=\"/{}/cards\" method=\"get\"><input type=\"hidden\" name=\"expand_all\" value=\"true\"{}><label class=\"filter-search\"><span class=\"for-screen-reader\">Filter these cards</span><input type=\"search\" name=\"terms[]\" value=\"{}\" placeholder=\"Filter these cards… [F]\" autocomplete=\"off\"></label>{preserved}<div class=\"quick-filters\" id=\"cards-filter-options\">{controls}</div><button class=\"more-filters__toggle\" type=\"button\" aria-label=\"{}\" title=\"{}\" aria-expanded=\"{}\" aria-controls=\"cards-filter-options\">☷</button>{clear}</form>",
        if expanded { " is-expanded" } else { "" },
        principal.slug,
        if expanded { "" } else { " disabled" },
        escape(query.terms.first().map(String::as_str).unwrap_or("")),
        if expanded {
            "Collapse filter options"
        } else {
            "Expand filter options"
        },
        if expanded {
            "Collapse filter options"
        } else {
            "Expand filter options"
        },
        expanded
    );
    let settings_link = query
        .board_ids
        .first()
        .filter(|_| query.board_ids.len() == 1)
        .filter(|id| boards.iter().any(|board| board.get::<String, _>("id") == id.as_str()))
        .map(|id| {
            format!(
                "<a class=\"settings-link\" href=\"/{}/boards/{}/edit\" aria-label=\"Settings for {}\" title=\"Settings for {}\">⚙</a>",
                principal.slug,
                escape(id),
                escape(&title),
                escape(&title)
            )
        })
        .unwrap_or_default();
    let values = HashMap::from([
        ("ACCOUNT", principal.slug.to_string()),
        ("TITLE", escape(&title)),
        ("FILTER_FORM", form),
        ("CARDS", list),
        ("COUNT", total.to_string()),
        ("PREVIOUS", previous),
        ("SETTINGS_LINK", settings_link),
        (
            "SHELL_STYLES",
            [
                ":root{--color-canvas:#101b20;--color-ink:#f4f5f6}",
                include_str!("../static/shell_footer.css"),
                include_str!("../static/notification_cards.css"),
                include_str!("../static/menu_overlay.css"),
            ]
            .concat(),
        ),
        (
            "SHELL_FOOTER",
            include_str!("../static/home_footer.html")
                .replace("{{ACCOUNT}}", &principal.slug.to_string()),
        ),
        ("SHELL_SCRIPT", include_str!("../static/events.js").into()),
    ]);
    Ok(Html(render_template(TEMPLATE, &values)))
}
