use std::collections::HashMap;

use axum::response::Html;

use super::{card_page::render_template, escape};

const TEMPLATE: &str = include_str!("../static/named-column.html");

pub(super) fn render(
    account: &str,
    board_id: &str,
    board_name: &str,
    column_id: &str,
    column_name: &str,
    page: i64,
    has_more: bool,
    previews: &str,
) -> Html<String> {
    let title = format!("Column: {column_name}");
    let frame = format!("cards_column_{column_id}");
    let content = if previews.is_empty() {
        "<div class=\"blank-slate blank-slate--default\">No cards here</div><div class=\"blank-slate blank-slate--drag overflow-ellipsis\">Drag cards here</div>".to_owned()
    } else {
        let next = if has_more {
            let next_page = page + 1;
            format!(
                "<a id=\"{frame}-pagination-link-{next_page}\" class=\"pagination-link pagination-link--active-when-observed\" aria-label=\"Load page {next_page}\" data-frame=\"{frame}-pagination-contents-{next_page}\" data-pagination-target=\"paginationLink\" href=\"/{account}/boards/{board_id}/columns/{column_id}?page={next_page}\">Load more…</a>",
                frame = escape(&frame),
                account = escape(account),
                board_id = escape(board_id),
                column_id = escape(column_id),
            )
        } else {
            String::new()
        };
        format!(
            "<div id=\"{frame}-pagination-list\" class=\"{frame} display-contents\" data-controller=\"pagination\" data-pagination-paginate-on-intersection-value=\"true\"><turbo-frame id=\"{frame}-pagination-contents-{page}\" role=\"presentation\">{previews}{next}</turbo-frame></div>",
            frame = escape(&frame),
        )
    };
    let values = HashMap::from([
        ("TITLE", escape(&title)),
        ("ACCOUNT", escape(account)),
        ("BOARD_ID", escape(board_id)),
        ("BOARD_NAME", escape(board_name)),
        ("FRAME", escape(&frame)),
        ("CONTENT", content),
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
            include_str!("../static/home_footer.html").replace("{{ACCOUNT}}", &escape(account)),
        ),
        ("SHELL_SCRIPT", include_str!("../static/events.js").into()),
    ]);
    Html(render_template(TEMPLATE, &values))
}
