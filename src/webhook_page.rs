use super::*;

const ACTION_LABELS: &[(&str, &str)] = &[
    ("card_published", "Card added"),
    ("card_title_changed", "Card title changed"),
    ("card_board_changed", "Card board changed"),
    ("comment_created", "Comment added"),
    ("card_assigned", "Card assigned"),
    ("card_unassigned", "Card unassigned"),
    ("card_triaged", "Card column changed"),
    ("card_closed", "Card moved to “Done”"),
    ("card_reopened", "Card reopened"),
    ("card_postponed", "Card moved to “Not Now”"),
    (
        "card_auto_postponed",
        "Card moved to “Not Now” due to inactivity",
    ),
    ("card_sent_back_to_triage", "Card moved back to “Maybe?”"),
];

fn label(action: &str) -> &str {
    ACTION_LABELS
        .iter()
        .find(|(key, _)| *key == action)
        .map(|(_, label)| *label)
        .unwrap_or(action)
}

fn page(
    account: i64,
    title: &str,
    content: &str,
    back_path: &str,
    back_label: &str,
    show_header_title: bool,
) -> Html<String> {
    let heading = if show_header_title {
        format!("<h1>{}</h1>", escape(title))
    } else {
        String::new()
    };
    signed_in_shell_html(
        account,
        title,
        &format!("<div class=\"webhook-content\">{content}</div>"),
        "",
        include_str!("../static/webhooks.css"),
        &format!(
            "<nav class=\"webhook-header\" aria-label=\"Webhook navigation\"><a href=\"{back_path}\">Back to {}</a>{heading}</nav>",
            escape(back_label)
        ),
    )
}

fn actions(selected: &[String]) -> String {
    let mut list = String::new();
    for (key, title) in ACTION_LABELS {
        if !WEBHOOK_ACTIONS.contains(key) {
            continue;
        }
        let checked = if selected.iter().any(|action| action == key) {
            " checked"
        } else {
            ""
        };
        list.push_str(&format!("<li><label><input type=\"checkbox\" name=\"webhook[subscribed_actions][]\" value=\"{key}\"{checked}><span>{}</span></label></li>",escape(title)));
    }
    format!(
        "<div class=\"checks\"><div class=\"checks__actions\"><button type=\"button\" data-checks=\"all\">Enable all</button> · <button type=\"button\" data-checks=\"none\">Disable all</button></div><ul>{list}</ul></div>"
    )
}

fn form(
    account: i64,
    board_id: &str,
    webhook: Option<&DbRow>,
    error: Option<&str>,
    submitted: Option<(&str, &str, &[String])>,
) -> Html<String> {
    let base = format!("/{account}/boards/{}/webhooks", escape(board_id));
    let (title, mut name, mut url, mut selected, action, method, submit, placeholder) =
        if let Some(row) = webhook {
            let id: String = row.get("id");
            let selected: Vec<String> =
                serde_json::from_str(&row.get::<String, _>("subscribed_actions"))
                    .unwrap_or_default();
            (
                row.get::<String, _>("name"),
                row.get::<String, _>("name"),
                None,
                selected,
                format!("{base}/{}", escape(&id)),
                "<input type=\"hidden\" name=\"_method\" value=\"put\">",
                "Save Changes",
                "Name your Webhook…",
            )
        } else {
            (
                "Set up a Webhook".to_owned(),
                String::new(),
                Some(String::new()),
                Vec::new(),
                base.clone(),
                "",
                "Create Webhook",
                "Name this Webhook…",
            )
        };
    if let Some((submitted_name, submitted_url, submitted_actions)) = submitted {
        name = submitted_name.to_owned();
        if url.is_some() {
            url = Some(submitted_url.to_owned());
        }
        selected = submitted_actions.to_vec();
    }
    let url_field = url.map(|value| format!("<div class=\"field\"><label for=\"webhook_url\"><strong>Payload URL</strong><small>This is the URL for the app that will receive payloads from Rizzy.</small></label><input id=\"webhook_url\" type=\"url\" name=\"webhook[url]\" value=\"{}\" pattern=\"https?://.*\" placeholder=\"https://example.com\" required></div>",escape(&value))).unwrap_or_default();
    let error = error
        .map(|message| format!("<p class=\"error\" role=\"alert\">{}</p>", escape(message)))
        .unwrap_or_default();
    let content = format!(
        "<article class=\"panel\">{error}<form method=\"post\" action=\"{action}\">{method}<div class=\"field\"><label for=\"webhook_name\">Webhook name</label><input id=\"webhook_name\" name=\"webhook[name]\" value=\"{}\" placeholder=\"{placeholder}\" required autofocus></div>{url_field}<div class=\"field\"><strong>Events</strong><small>Trigger a call to the Payload URL when:</small>{}</div><button class=\"submit\" type=\"submit\">{submit}</button></form></article><script>document.querySelectorAll('[data-checks]').forEach(button=>button.addEventListener('click',()=>document.querySelectorAll('.checks input[type=checkbox]').forEach(input=>input.checked=button.dataset.checks==='all')))</script>",
        escape(&name),
        actions(&selected)
    );
    let (back_path, back_label, show_header_title) = if let Some(row) = webhook {
        (action, row.get::<String, _>("name"), false)
    } else {
        (base, "Webhooks".to_owned(), true)
    };
    page(
        account,
        &title,
        &content,
        &back_path,
        &back_label,
        show_header_title,
    )
}

pub(super) async fn new(
    State(app): Shared,
    Path((account, board_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Html<String>> {
    let p = principal(&app, &headers, &account).await?;
    board(&app, &p, &board_id).await?;
    p.account_admin()?;
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok(form(p.slug, &board_id, None, None, None))
}

pub(super) async fn edit(
    State(app): Shared,
    Path((account, board_id, webhook_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Html<String>> {
    let p = principal(&app, &headers, &account).await?;
    board(&app, &p, &board_id).await?;
    p.account_admin()?;
    let webhook = webhook_row(&app, &p, &board_id, &webhook_id).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    Ok(form(p.slug, &board_id, Some(&webhook), None, None))
}

pub(super) fn invalid_form(
    account: i64,
    board_id: &str,
    webhook: Option<&DbRow>,
    error: &str,
    name: &str,
    url: &str,
    actions: &[String],
) -> Html<String> {
    form(
        account,
        board_id,
        webhook,
        Some(error),
        Some((name, url, actions)),
    )
}

pub(super) fn index(
    account: i64,
    board_id: &str,
    board_name: &str,
    rows: &[DbRow],
    page_number: i64,
    total: i64,
    offset: i64,
    limit: i64,
) -> Html<String> {
    let base = format!("/{account}/boards/{}", escape(board_id));
    let mut entries = String::new();
    for row in rows {
        let id: String = row.get("id");
        let name: String = row.get("name");
        let path = format!("{base}/webhooks/{}", escape(&id));
        entries.push_str(&format!("<li><a href=\"{path}\"><strong>{}</strong></a><form method=\"post\" action=\"{path}\"><input type=\"hidden\" name=\"_method\" value=\"delete\"><button type=\"submit\" aria-label=\"Remove {}\" onclick=\"return confirm('Remove this webhook?')\">Remove</button></form></li>",escape(&name),escape(&name)));
    }
    let intro = if rows.is_empty() {
        "<p>Webhooks can notify another application when something happens in this Rizzy board. You’ll choose which events to subscribe to and provide a URL to receive the data.</p><p>For example, you could create a webhook that posts to a Campfire chat in Basecamp when new cards are added to Rizzy.</p>"
    } else {
        ""
    };
    let before = if page_number > 1 {
        format!(
            "<a href=\"{base}/webhooks?page={}\">Previous</a>",
            page_number - 1
        )
    } else {
        String::new()
    };
    let after = if total > offset + limit {
        format!(
            "<a href=\"{base}/webhooks?page={}\">Next</a>",
            page_number + 1
        )
    } else {
        String::new()
    };
    let content = format!(
        "<section class=\"panel\">{intro}<ul class=\"webhook-list\">{entries}</ul><nav class=\"pagination\">{before}{after}</nav><a class=\"submit\" href=\"{base}/webhooks/new\">Set up a new webhook</a></section>"
    );
    page(account, "Webhooks", &content, &base, board_name, true)
}

pub(super) fn show(
    account: i64,
    board_id: &str,
    row: &DbRow,
    deliveries: &[DbRow],
) -> Html<String> {
    let id: String = row.get("id");
    let path = format!(
        "/{account}/boards/{}/webhooks/{}",
        escape(board_id),
        escape(&id)
    );
    let name: String = row.get("name");
    let url: String = row.get("url");
    let secret: String = row.get("signing_secret");
    let actions: Vec<String> =
        serde_json::from_str(&row.get::<String, _>("subscribed_actions")).unwrap_or_default();
    let actions_html = if actions.is_empty() {
        "<p>This Webhook isn't subscribed to any events. It will never trigger.</p>".to_owned()
    } else {
        format!(
            "<ul>{}</ul>",
            actions
                .iter()
                .map(|action| format!("<li>{}</li>", escape(label(action))))
                .collect::<String>()
        )
    };
    let delivery_html = if deliveries.is_empty() {
        "<p>This Webhook hasn't been triggered yet</p>".to_owned()
    } else {
        format!(
            "<ul>{}</ul>",
            deliveries
                .iter()
                .map(|delivery| format!(
                    "<li><strong>{}</strong> <time>{}</time></li>",
                    escape(&delivery.get::<String, _>("state")),
                    escape(&delivery.get::<String, _>("created_at"))
                ))
                .collect::<String>()
        )
    };
    let reactivate = if row.get::<i64, _>("active") == 0 {
        format!(
            "<form method=\"post\" action=\"{path}/activation\"><button type=\"submit\">Reactivate</button></form>"
        )
    } else {
        String::new()
    };
    let content = format!(
        "<section class=\"panel\"><a href=\"{path}/edit\">Edit</a><h2>{}</h2><p class=\"url\">{}</p>{reactivate}<h3>Secret</h3><strong><code>{}</code></strong><p>We'll send an <code>X-Webhook-Signature</code> header with each request. Generate an HMAC using SHA256 of the request body with this secret to verify the request.</p><h3>Subscribed to</h3>{actions_html}<h3>Deliveries</h3>{delivery_html}</section>",
        escape(&name),
        escape(&url),
        escape(&secret)
    );
    page(
        account,
        &name,
        &content,
        &format!("/{account}/boards/{}/webhooks", escape(board_id)),
        "Webhooks",
        false,
    )
}
