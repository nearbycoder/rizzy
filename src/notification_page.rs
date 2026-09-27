use serde_json::Value;

use super::escape;

fn field<'a>(item: &'a Value, key: &str) -> &'a str {
    item.get(key).and_then(Value::as_str).unwrap_or("")
}

pub(super) fn item(account: i64, notification: &Value) -> String {
    let id = escape(field(notification, "id"));
    let card = &notification["card"];
    let creator = &notification["creator"];
    let read = notification["read"].as_bool().unwrap_or(false);
    let number = card["number"].as_i64().unwrap_or_default();
    let card_url = format!("/{account}/cards/{number}");
    let reading_url = format!("/{account}/notifications/{id}/reading");
    let method = if read {
        "<input type=\"hidden\" name=\"_method\" value=\"delete\">"
    } else {
        ""
    };
    let action = if read {
        "Mark as unread"
    } else {
        "Mark as read"
    };
    let unread_count = notification["unread_count"].as_i64().unwrap_or(0);
    let badge = if unread_count > 1 {
        format!("<span class=\"badge-count\">{unread_count}</span>")
    } else {
        String::new()
    };
    let classes = format!(
        "card card--notification{}{}",
        if card["closed"].as_bool() == Some(true) {
            " card--closed"
        } else {
            ""
        },
        if read { "" } else { " unread" },
    );
    let creator_name = field(creator, "name");
    let creator_id = escape(field(creator, "id"));
    let title = escape(field(notification, "title"));
    let title_html = if field(notification, "source_type") == "mention" {
        format!(
            "<strong class=\"card__title overflow-ellipsis\"><mark class=\"card__notification-mentioner\">{title}</mark></strong>"
        )
    } else {
        format!("<div class=\"card__title overflow-ellipsis\">{title}</div>")
    };
    format!(
        "<div id=\"notification_{id}\" class=\"tray__item tray__item--notification\" data-card-id=\"{}\" data-navigable-list-target=\"item\"><a href=\"{card_url}\" class=\"{classes}\" data-turbo-frame=\"_top\"><header class=\"card__header\"><div class=\"card__board\"><span class=\"card__id\"><span class=\"for-screen-reader\">Card number</span>{number}</span><span class=\"card__board-name\"><span class=\"overflow-ellipsis\">{}</span></span></div><div class=\"card__notification-meta overflow-ellipsis flex-item-grow flex-item-no-shrink txt-align-end\"><span class=\"card__creator\">{}</span></div><div class=\"card__notification-meta overflow-ellipsis flex-item-no-shrink\"><span class=\"card__timestamp\"><time datetime=\"{}\">{}</time></span></div><div class=\"card__notification-meta flex-item-no-shrink\"><form action=\"{reading_url}\" method=\"post\" data-notification-toggle>{method}<button type=\"submit\" class=\"card__notification-unread-indicator btn btn--circle borderless\" title=\"{action}\" aria-label=\"{action}\">{}{badge}</button></form></div></header><div class=\"card__body\"><div class=\"avatar txt-x-small\"><img src=\"/{account}/users/{creator_id}/avatar\" alt=\"\" aria-hidden=\"true\" width=\"48\" height=\"48\"></div><h3 class=\"flex flex-column min-width flex-item-grow font-weight-normal\">{title_html}<div class=\"card__notification-body overflow-ellipsis\">{}</div></h3></div></a></div>",
        escape(card["id"].as_str().unwrap_or("")),
        escape(card["board_name"].as_str().unwrap_or("")),
        escape(&super::board_views::public_familiar_name(creator_name)),
        escape(field(notification, "created_at")),
        escape(field(notification, "created_at")),
        if read { "◌" } else { "×" },
        escape(field(notification, "body")),
    )
}

pub(super) fn tray(account: i64, notifications: &[Value]) -> String {
    let items = notifications
        .iter()
        .map(|notice| item(account, notice))
        .collect::<String>();
    format!("<turbo-frame id=\"notifications\">{items}</turbo-frame>")
}

pub(super) fn reading_stream(account: i64, notification: &Value, read: bool) -> String {
    let id = escape(field(notification, "id"));
    let list = if read {
        "notifications_list_read"
    } else {
        "notifications_list"
    };
    let item = item(account, notification);
    format!(
        "<turbo-stream action=\"remove\" target=\"notification_{id}\"></turbo-stream>\n<turbo-stream action=\"prepend\" target=\"{list}\"><template>{item}</template></turbo-stream>\n"
    )
}

pub(super) fn index(
    account: i64,
    unread: &[Value],
    read: &[Value],
    page: i64,
    total_read: i64,
    next_limit: i64,
    hidden_unread: i64,
    show_unread: bool,
) -> String {
    let unread_html = unread
        .iter()
        .map(|notice| item(account, notice))
        .collect::<String>();
    let read_html = read
        .iter()
        .map(|notice| item(account, notice))
        .collect::<String>();
    let unread_heading = if unread.is_empty() {
        "<div class=\"notifications-list__blank-slate blank-slate\">Nothing new for you</div>"
            .to_string()
    } else {
        format!(
            "<div class=\"notifications-list__heading\"><h2>New for you</h2><form method=\"post\" action=\"/{account}/notifications/bulk_reading\"><button type=\"submit\">Mark all as read</button></form></div>"
        )
    };
    let hidden = if hidden_unread > 0 {
        format!(
            "<p class=\"notifications-list__overflow\">Showing the 500 most recent ({hidden_unread} are hidden)</p>"
        )
    } else {
        String::new()
    };
    let unread_section = if show_unread {
        format!(
            "<section class=\"notifications-list panel panel--wide\"><div id=\"unread_section\">{unread_heading}<div id=\"notifications_list\" contents>{unread_html}</div>{hidden}</div></section>"
        )
    } else {
        String::new()
    };
    let read_section = if read.is_empty() {
        String::new()
    } else {
        format!(
            "<section class=\"notifications-list notifications-list--read panel panel--wide\" id=\"read_section\"><h2>Previously seen</h2><div id=\"notifications_list_read\" contents>{read_html}</div></section>"
        )
    };
    let next = if next_limit < total_read {
        format!(
            "<div id=\"next_page\"><a href=\"/{account}/notifications?page={}\">Load more</a></div>",
            page + 1
        )
    } else {
        String::new()
    };
    let html = format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Notifications | Rizzy</title><style>:root{{font:16px/1.5 system-ui,sans-serif;background:#f6f4ef;color:#24211e}}*{{box-sizing:border-box}}body{{margin:0}}a{{color:inherit}}button{{font:inherit;cursor:pointer}}.header{{display:flex;align-items:center;justify-content:space-between;gap:16px;padding:18px max(20px,calc((100vw - 920px)/2));background:#fff;border-bottom:1px solid #e5e0d8}}.header h1{{font-size:1.5rem;margin:0}}.header a{{text-decoration:none}}main{{max-width:920px;margin:28px auto;padding:0 20px}}.notifications-list{{background:#fff;border-radius:16px;padding:20px;margin-bottom:20px;box-shadow:0 2px 16px #0000000c}}.notifications-list__heading{{display:flex;justify-content:space-between;align-items:center}}h2{{font-size:.9rem;text-transform:uppercase;letter-spacing:.08em;color:#ab4937}}.notifications-list--read h2{{color:#79736a}}.notifications-list__blank-slate{{text-align:center;padding:34px;color:#777}}.tray__item{{border-top:1px solid #eee8df;padding:12px 0}}.card__header{{display:flex;gap:12px;align-items:center;color:#777;font-size:.8rem}}.card__board{{display:flex;gap:8px;margin-right:auto}}.card__id{{font-weight:700}}.card__creator{{white-space:nowrap}}.card__timestamp{{white-space:nowrap}}.card__notification-unread-indicator{{border:0;background:transparent;padding:6px;color:#9b4933}}.card__body{{display:flex;flex-direction:column;gap:2px;text-decoration:none;padding:6px 0}}.card__title{{font-weight:700}}.card__notification-body{{color:#6e675f;overflow-wrap:anywhere}}.unread .card__title{{color:#201c19}}.badge-count{{margin-left:4px}}.for-screen-reader{{position:absolute;clip:rect(0,0,0,0);width:1px;height:1px;overflow:hidden}}#next_page{{text-align:center;padding:20px}}#next_page a,button{{border:1px solid #d1c9bd;border-radius:8px;background:#fff;padding:7px 11px}}@media(max-width:650px){{.card__creator,.card__timestamp{{display:none}}}}</style></head><body><header class="header"><a href="/{account}/">← Home</a><h1>Notifications</h1><a href="/{account}/notifications/settings">Notification settings</a></header><main data-controller="badge navigable-list">{unread_section}{read_section}{next}</main><script>{}</script></body></html>"##,
        include_str!("../assets/notifications.js")
    );
    html.replace(
        "</style></head>",
        &format!(
            "</style><style>{}</style></head>",
            include_str!("../static/notification_cards.css")
        ),
    )
}

pub(super) fn next_page_stream(account: i64, read: &[Value], page: i64, has_next: bool) -> String {
    let items = read
        .iter()
        .map(|notice| item(account, notice))
        .collect::<String>();
    let next = if has_next {
        format!(
            "<div id=\"next_page\"><a href=\"/{account}/notifications?page={}\">Load more</a></div>",
            page + 1
        )
    } else {
        String::new()
    };
    format!(
        "<turbo-stream action=\"append\" target=\"notifications_list_read\"><template>{items}</template></turbo-stream><turbo-stream action=\"replace\" target=\"next_page\"><template>{next}</template></turbo-stream>"
    )
}
