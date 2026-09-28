use axum::{
    extract::Path,
    http::{HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use sqlx::{Row, any::AnyRow as DbRow};
use std::collections::HashMap;

use super::{
    App, Principal, Result, assignment_page, board, board_views, card_controls, comment_edit_page,
    db_err, err, escape, reaction_page, step_frame, storage, tagging_page, wrap_html,
};

const TEMPLATE: &str = include_str!("../static/card.html");
const NEW_COMMENT: &str = include_str!("../static/new_comment.html");

pub(super) async fn icon(Path(filename): Path<String>) -> Response {
    let svg = match filename.as_str() {
        "arrow-left.svg" => include_str!("../static/card-icons/arrow-left.svg"),
        "bell-off.svg" => include_str!("../static/card-icons/bell-off.svg"),
        "bell.svg" => include_str!("../static/card-icons/bell.svg"),
        "check.svg" => include_str!("../static/card-icons/check.svg"),
        "golden-ticket.svg" => include_str!("../static/card-icons/golden-ticket.svg"),
        "pencil.svg" => include_str!("../static/card-icons/pencil.svg"),
        "picture-add.svg" => include_str!("../static/card-icons/picture-add.svg"),
        "picture-remove.svg" => include_str!("../static/card-icons/picture-remove.svg"),
        "pinned.svg" => include_str!("../static/card-icons/pinned.svg"),
        "tag.svg" => include_str!("../static/card-icons/tag.svg"),
        "trash.svg" => include_str!("../static/card-icons/trash.svg"),
        "unpinned.svg" => include_str!("../static/card-icons/unpinned.svg"),
        _ => return StatusCode::NOT_FOUND.into_response(),
    };
    (
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("image/svg+xml"),
            ),
            (
                header::CACHE_CONTROL,
                HeaderValue::from_static("public, max-age=300"),
            ),
        ],
        svg,
    )
        .into_response()
}

pub(super) fn new_comment_form(principal: &Principal, card: &DbRow, first_comment: bool) -> String {
    let placeholder = if first_comment && card.get::<String, _>("creator_id") == principal.user_id {
        "Next, add some notes, context, pictures, or video about this…"
    } else {
        "Type your comment…"
    };
    let values = HashMap::from([
        ("ACCOUNT", principal.slug.to_string()),
        ("NUMBER", card.get::<i64, _>("number").to_string()),
        ("CARD_ID", escape(&card.get::<String, _>("id"))),
        ("BOARD_ID", escape(&card.get::<String, _>("board_id"))),
        ("USER_ID", escape(&principal.user_id)),
        ("COMMENT_PLACEHOLDER", placeholder.to_owned()),
    ]);
    render_template(NEW_COMMENT, &values)
}

const EDIT_HEAD: &str = r#"<link rel="stylesheet" href="/assets/lexxy/lexxy.css"><link rel="stylesheet" href="/assets/lexxy/rizzy-dark.css"><script type="importmap">{"imports":{"@rails/activestorage":"/assets/lexxy/activestorage.esm.js"}}</script><script type="module" src="/assets/lexxy/lexxy.js"></script>"#;

pub(super) fn edit_frame(principal: &Principal, card: &DbRow) -> String {
    let card_id: String = card.get("id");
    let board_id: String = card.get("board_id");
    let number: i64 = card.get("number");
    let title: String = card.get("title");
    let description: String = card.get("description");
    let path = format!("/{}/cards/{number}", principal.slug);
    format!(
        r##"<turbo-frame id="edit_card_{card_id}"><turbo-stream action="update" target="card_closure_toggle_card_{card_id}"><template><button type="submit" form="edit_form_card_{card_id}">Save changes</button></template></turbo-stream><form id="edit_form_card_{card_id}" action="{path}" method="post" data-local-save-key-value="card-{card_id}"><input type="hidden" name="_method" value="patch"><h1 class="card__title"><textarea id="card_title" name="card[title]" aria-label="Name it…" placeholder="Name it…" maxlength="255" required autofocus>{title}</textarea></h1><lexxy-editor id="card_description" name="card[description]" value="{description}" placeholder="Add some notes, context, pictures, or video about this…" data-direct-upload-url="/{account}/rails/active_storage/direct_uploads" data-blob-url-template="/{account}/rails/active_storage/blobs/redirect/:signed_id/:filename"><lexxy-prompt trigger="@" src="/{account}/prompts/boards/{board_id}/users" name="mention"></lexxy-prompt><lexxy-prompt trigger="#" src="/{account}/prompts/cards" name="card" insert-editable-text="true" remote-filtering="true" supports-space-in-searches="true"></lexxy-prompt></lexxy-editor><button type="submit">Save changes</button><a href="{path}">Close editor and discard changes</a><p role="alert" class="card-edit__error" id="edit-error"></p></form></turbo-frame>"##,
        path = path,
        card_id = escape(&card_id),
        title = escape(&title),
        description = escape(&description),
        account = principal.slug,
        board_id = escape(&board_id),
    )
}

pub(super) fn edit(principal: &Principal, card: &DbRow) -> Html<String> {
    let content = format!(
        "<div class=\"card-edit\">{frame}</div><script>{draft_script}</script><script>{script}</script>",
        frame = edit_frame(principal, card),
        draft_script = include_str!("../static/card_edit_draft.js"),
        script = include_str!("../static/card_edit.js"),
    );
    super::signed_in_shell_html(
        principal.slug,
        "Edit card",
        &content,
        EDIT_HEAD,
        include_str!("../static/card_edit.css"),
        "",
    )
}

pub(super) async fn update_stream(
    app: &App,
    principal: &Principal,
    card: &DbRow,
) -> Result<String> {
    let card_id: String = card.get("id");
    let number: i64 = card.get("number");
    let title: String = card.get("title");
    let description: String = card.get("description");
    let (_, description_html, _) =
        storage::render_rich_text(app, &principal.account_id, principal.slug, &description).await?;
    let content = format!(
        "<h1 class=\"card__title\"><a class=\"card__title-link\" href=\"/{}/cards/{number}/edit\">{}</a></h1><div class=\"card__description lexxy-content\">{description_html}</div>",
        principal.slug,
        board_views::public_html_title(&title)
    );
    let draft = card.get::<String, _>("status") == "drafted";
    let container = if draft {
        let image = storage::image_url_for_card(app, principal, &card_id)
            .await?
            .as_str()
            .map(|url| {
                format!(
                    "<img class=\"card-perma__image\" src=\"{}\" alt=\"\">",
                    escape(url)
                )
            })
            .unwrap_or_default();
        format!(
            "<section id=\"card_container_card_{card_id}\" class=\"card-perma card-perma--draft\">{image}<div class=\"card__content\">{content}</div></section>"
        )
    } else {
        let page = show(app, principal, card, "UTC").await?.0;
        let article = page
            .find("<article class=\"card-perma")
            .and_then(|start| {
                page[start..]
                    .find("</article>")
                    .map(|end| &page[start..start + end + "</article>".len()])
            })
            .unwrap_or("");
        format!("<section id=\"card_container_card_{card_id}\">{article}</section>")
    };
    let closure = if card.get::<i64, _>("closed") != 0 {
        "<span>Completed</span>".to_owned()
    } else if draft {
        String::new()
    } else {
        "<button type=\"button\" id=\"closure\">Mark as done</button>".to_owned()
    };
    Ok(format!(
        "<turbo-stream action=\"replace\" method=\"morph\" target=\"card_container_card_{card_id}\"><template>{container}</template></turbo-stream><turbo-stream action=\"update\" target=\"edit_card_{card_id}\"><template>{content}</template></turbo-stream><turbo-stream action=\"replace\" target=\"card_closure_toggle_card_{card_id}\"><template><div id=\"card_closure_toggle_card_{card_id}\">{closure}</div></template></turbo-stream>"
    ))
}

pub(super) async fn flag_stream(app: &App, principal: &Principal, card: &DbRow) -> Result<String> {
    let card_id: String = card.get("id");
    let page = show(app, principal, card, "UTC").await?.0;
    let container_start = format!("<section id=\"card_container_card_{card_id}\">");
    let container = page
        .find(&container_start)
        .and_then(|start| {
            page[start..]
                .find("</article></section>")
                .map(|end| &page[start..start + end + "</article></section>".len()])
        })
        .ok_or_else(|| err(StatusCode::INTERNAL_SERVER_ERROR, "Card container missing"))?;
    let toggle_start = format!("<div id=\"card_closure_toggle_card_{card_id}\"");
    let toggle = page
        .find(&toggle_start)
        .and_then(|start| {
            page[start..]
                .find("\n  <section id=\"column-picker\"")
                .map(|end| page[start..start + end].trim())
        })
        .ok_or_else(|| {
            err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Card closure control missing",
            )
        })?;
    Ok(format!(
        "<turbo-stream method=\"morph\" action=\"replace\" target=\"card_container_card_{card_id}\"><template>{container}</template></turbo-stream><turbo-stream action=\"replace\" target=\"card_closure_toggle_card_{card_id}\"><template>{toggle}</template></turbo-stream>"
    ))
}

pub(super) fn render_template(template: &str, values: &HashMap<&str, String>) -> String {
    let mut result = String::with_capacity(template.len() + 2048);
    let mut rest = template;
    while let Some((before, after_open)) = rest.split_once("{{") {
        result.push_str(before);
        if let Some((key, after_close)) = after_open.split_once("}}") {
            if let Some(value) = values.get(key) {
                result.push_str(value);
            } else {
                result.push_str("{{");
                result.push_str(key);
                result.push_str("}}");
            }
            rest = after_close;
        } else {
            result.push_str("{{");
            rest = after_open;
            break;
        }
    }
    result.push_str(rest);
    result
}

pub(super) async fn show(
    app: &App,
    principal: &Principal,
    card: &DbRow,
    timezone: &str,
) -> Result<Html<String>> {
    let card_id: String = card.get("id");
    let board_id: String = card.get("board_id");
    let number: i64 = card.get("number");
    let title: String = card.get("title");
    let description: String = card.get("description");
    let (plain_description, description_html, _) =
        storage::render_rich_text(app, &principal.account_id, principal.slug, &description).await?;
    let card_board = board(app, principal, &board_id).await?;
    let board_name: String = card_board.get("name");
    let closed = card.get::<i64, _>("closed") != 0;
    let postponed = card.get::<i64, _>("postponed") != 0;
    let completion = if closed {
        sqlx::query("SELECT z.created_at,u.name FROM closures z LEFT JOIN users u ON u.id=z.user_id WHERE z.card_id=?")
            .bind(&card_id)
            .fetch_optional(&app.db)
            .await
            .map_err(db_err)?
    } else {
        None
    };
    let not_now = if postponed {
        sqlx::query("SELECT n.created_at,u.name,u.role FROM card_not_nows n LEFT JOIN users u ON u.id=n.user_id WHERE n.card_id=?")
            .bind(&card_id)
            .fetch_optional(&app.db)
            .await
            .map_err(db_err)?
    } else {
        None
    };
    let steps = sqlx::query(
        "SELECT id,content,completed FROM steps WHERE card_id=? ORDER BY created_at,id",
    )
    .bind(&card_id)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let mut step_html = String::new();
    step_html.push_str(
        "<section aria-label=\"Steps\"><ol class=\"steps txt-small margin-block-start-auto\">",
    );
    for step in steps {
        let id: String = step.get("id");
        let content: String = step.get("content");
        step_html.push_str(&step_frame(
            &principal.slug.to_string(),
            &number.to_string(),
            &id,
            &content,
            step.get::<i64, _>("completed") != 0,
        ));
    }
    if !closed {
        step_html.push_str(&format!("<li id=\"new_step_card_{}\" class=\"step step--new\"><input type=\"checkbox\" class=\"checkbox\" disabled><form class=\"min-width steps__form\" id=\"step-form\" action=\"/{}/cards/{}/steps\" method=\"post\" data-controller=\"form\" data-action=\"submit-&gt;form#preventEmptySubmit submit-&gt;form#preventComposingSubmit turbo:submit-end-&gt;form#reset\"><input id=\"new-step\" name=\"step[content]\" class=\"input step__content hide-focus-ring\" placeholder=\"Add a step…\" autocomplete=\"off\" data-form-target=\"input\" data-1p-ignore=\"true\" data-action=\"compositionstart-&gt;form#compositionStart compositionend-&gt;form#compositionEnd\" aria-label=\"Add a step\"></form></li>", escape(&card_id), principal.slug, number));
    }
    step_html.push_str("</ol></section>");
    let comments = sqlx::query("SELECT c.id,c.body,c.created_at,c.updated_at,c.creator_id,u.name AS creator_name,u.role AS creator_role FROM comments c JOIN users u ON u.id=c.creator_id WHERE c.card_id=? ORDER BY c.created_at,c.id")
        .bind(&card_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let latest_comment_id = comments
        .iter()
        .map(|comment| comment.get::<String, _>("id"))
        .max()
        .unwrap_or_default();
    let first_comment = comments.is_empty();
    let rows = sqlx::query("SELECT r.id,r.content,r.reacter_id,r.reactable_id,u.name FROM reactions r JOIN users u ON u.id=r.reacter_id WHERE r.account_id=? AND ((r.reactable_type='Card' AND r.reactable_id=?) OR (r.reactable_type='Comment' AND r.reactable_id IN (SELECT id FROM comments WHERE card_id=?))) ORDER BY r.created_at,r.id")
        .bind(&principal.account_id)
        .bind(&card_id)
        .bind(&card_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let mut reaction_rows = HashMap::<String, Vec<DbRow>>::new();
    for row in rows {
        reaction_rows
            .entry(row.get::<String, _>("reactable_id"))
            .or_default()
            .push(row);
    }
    let card_reactions = reaction_page::reaction_frame(
        principal,
        card,
        None,
        reaction_rows
            .get(&card_id)
            .map(Vec::as_slice)
            .unwrap_or(&[]),
    );
    let mut comment_html = String::new();
    for comment in comments {
        let comment_id: String = comment.get("id");
        let body: String = comment.get("body");
        let content = if comment.get::<String, _>("creator_role") == "system" {
            wrap_html(&body)
        } else {
            storage::render_rich_text(app, &principal.account_id, principal.slug, &body)
                .await?
                .1
        };
        let reactions = reaction_page::reaction_frame(
            principal,
            card,
            Some(&comment_id),
            reaction_rows
                .get(&comment_id)
                .map(Vec::as_slice)
                .unwrap_or(&[]),
        );
        comment_html.push_str(&comment_edit_page::comment_frame(
            principal.slug,
            number,
            &comment_id,
            &comment.get::<String, _>("creator_id"),
            &comment.get::<String, _>("creator_name"),
            &comment.get::<String, _>("creator_role"),
            &comment.get::<String, _>("created_at"),
            &comment.get::<String, _>("updated_at"),
            &content,
            &reactions,
            comment.get::<String, _>("creator_id") == principal.user_id,
        ));
    }
    let watching: i64 = sqlx::query_scalar(
        "SELECT COALESCE((SELECT watching FROM watches WHERE card_id=? AND user_id=?),0)",
    )
    .bind(&card_id)
    .bind(&principal.user_id)
    .fetch_one(&app.db)
    .await
    .map_err(db_err)?;
    let pinned: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM pins WHERE card_id=? AND user_id=?")
        .bind(&card_id)
        .bind(&principal.user_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let image_url = storage::image_url_for_card(app, principal, &card_id).await?;
    let image = image_url
        .as_str()
        .map(|url| {
            format!(
                "<img class=\"card-perma__image\" src=\"{}\" alt=\"\">",
                escape(url)
            )
        })
        .unwrap_or_default();
    let image_button = if image_url.as_str().is_some() {
        "<button class=\"side-action side-action--image\" id=\"remove-image\" type=\"button\" aria-label=\"Remove background image\" title=\"Remove background image\"><span class=\"card-icon card-icon--picture-remove\" aria-hidden=\"true\"></span></button>".to_owned()
    } else if closed {
        String::new()
    } else {
        format!(
            "<form id=\"image-upload-form\" action=\"/{}/cards/{number}\" method=\"post\" enctype=\"multipart/form-data\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><label class=\"side-action side-action--image\" title=\"Add background image\"><span class=\"for-screen-reader\">Add background image</span><span class=\"card-icon card-icon--picture-add\" aria-hidden=\"true\"></span><input id=\"image-upload\" name=\"card[image]\" type=\"file\" accept=\"image/png,image/jpeg,image/jpg,image/webp\" aria-label=\"Add background image\"></label><noscript><button type=\"submit\">Save background image</button></noscript></form>",
            principal.slug
        )
    };
    let tag_control = tagging_page::tags(app, principal, card).await?;
    let closure_notice = if closed {
        let (name, timestamp) = completion
            .as_ref()
            .map(|row| {
                (
                    row.get::<Option<String>, _>("name").unwrap_or_default(),
                    row.get::<String, _>("created_at"),
                )
            })
            .unwrap_or_default();
        let seconds = chrono::DateTime::parse_from_rfc3339(&timestamp)
            .map(|value| value.timestamp())
            .unwrap_or_default();
        format!(
            "<div class=\"card-perma__closure-message card-perma__closure-message--completed\"><span>Completed by {} on <time datetime=\"{seconds}\" data-local-time-target=\"shortdate\">&nbsp;</time>.</span><form id=\"undo-closure-form\" method=\"post\" action=\"/{}/cards/{number}/closure\"><input type=\"hidden\" name=\"_method\" value=\"delete\"><button id=\"undo-closure\" type=\"submit\">Undo</button></form></div>",
            escape(&name),
            principal.slug
        )
    } else if !postponed {
        let last_active_at: String = card.get("last_active_at");
        chrono::DateTime::parse_from_rfc3339(&last_active_at)
            .ok()
            .and_then(|active| {
                active.checked_add_signed(chrono::Duration::days(
                    card_board.get::<i64, _>("auto_postpone_period_in_days"),
                ))
            })
            .map(|deadline| {
                format!(
                    "<div class=\"card-perma__closure-message\" id=\"closure_notice_card_{}\">Moves to “Not Now” <time datetime=\"{}\" data-local-time-target=\"indays\">&nbsp;</time> if there’s no activity.</div>",
                    escape(&card_id),
                    deadline.timestamp()
                )
            })
            .unwrap_or_default()
    } else {
        String::new()
    };
    let stamp = completion
        .as_ref()
        .map(|row| ("Done", row, false))
        .or_else(|| not_now.as_ref().map(|row| ("Not Now", row, true)))
        .map(|(label, row, is_not_now)| {
            let timestamp: String = row.get("created_at");
            let name = row.get::<Option<String>, _>("name").unwrap_or_default();
            let familiar = board_views::public_familiar_name(&name);
            let date = board_views::public_stamp_date(&timestamp);
            format!("<div class=\"card__closed{}\"><span class=\"card__closed-title\" data-text=\"{label}\">{label}</span><strong class=\"card__closed-date\">{date}</strong><span class=\"card__closed-by-line\">by <span class=\"card__closed-by\">{}</span></span></div>", if is_not_now && row.get::<Option<String>, _>("role").as_deref() == Some("system") { " card__closed--system" } else { "" }, escape(&familiar))
        })
        .unwrap_or_default();
    let column_id: Option<String> = card.get("column_id");
    let columns = sqlx::query("SELECT id,name FROM columns WHERE board_id=? ORDER BY position,id")
        .bind(&board_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let mut stage_rail =
        String::from("<div class=\"stage-rail\" role=\"radiogroup\" aria-label=\"Card stage\">");
    let mut stage = |name: &str, label: &str, current: bool, id: Option<&str>| {
        let label_html = if name == "done" {
            format!(
                "<svg class=\"stage-rail__mobile-icon\" viewBox=\"0 0 24 24\" aria-hidden=\"true\" focusable=\"false\"><path d=\"m23.146 5.4-2.792-2.8a.5.5 0 0 0-.708 0L7.854 14.4a.5.5 0 0 1-.708 0l-2.792-2.8a.5.5 0 0 0-.708 0L.854 14.4a.5.5 0 0 0 0 .707L7.146 21.4a.5.5 0 0 0 .708 0L23.146 6.1a.5.5 0 0 0 0-.7\"/></svg>{}",
                escape(label)
            )
        } else {
            escape(label)
        };
        stage_rail.push_str(&format!("<button type=\"button\" role=\"radio\" aria-checked=\"{}\" class=\"stage-rail__option{}\" data-stage=\"{}\"{}>{label_html}</button>",if current {"true"} else {"false"},if current {" is-current"} else {""},name,id.map(|id|format!(" data-column-id=\"{}\"",escape(id))).unwrap_or_default()));
    };
    stage("not_now", "Not Now", postponed, None);
    stage(
        "maybe",
        "Maybe?",
        !closed && !postponed && column_id.is_none(),
        None,
    );
    for column in columns {
        let id: String = column.get("id");
        let name: String = column.get("name");
        stage(
            "column",
            &name,
            !closed && !postponed && column_id.as_deref() == Some(&id),
            Some(&id),
        );
    }
    stage("done", "Done", closed, None);
    stage_rail.push_str("</div>");
    let mut values = HashMap::from([
        ("ACCOUNT", principal.slug.to_string()),
        ("NUMBER", number.to_string()),
        ("TIMEZONE", escape(timezone)),
        ("CARD_ID", escape(&card_id)),
        (
            "CARD_REVISION",
            escape(&format!(
                "{}:{}",
                card.get::<String, _>("updated_at"),
                latest_comment_id
            )),
        ),
        ("BOARD_ID", escape(&board_id)),
        ("BOARD_NAME", escape(&board_name)),
        ("TITLE", escape(&title)),
        ("TITLE_HTML", board_views::public_html_title(&title)),
        ("DESCRIPTION_HTML", description_html),
        (
            "DESCRIPTION_CLASS",
            if plain_description.trim().is_empty() {
                " is-description-empty"
            } else {
                ""
            }
            .to_owned(),
        ),
        ("DESCRIPTION_VALUE", escape(&description)),
        (
            "ASSIGNMENT_META",
            assignment_page::meta(app, principal, card).await?,
        ),
        ("COMMENTS", comment_html),
        ("REACTIONS", card_reactions),
        (
            "NEW_COMMENT_FORM",
            new_comment_form(principal, card, first_comment),
        ),
        (
            "COMMENT_WATCHERS",
            super::card_controls::watcher_section(app, principal, &card_id).await?,
        ),
        ("STEPS", step_html),
        ("STAGE_RAIL", stage_rail),
        (
            "GOLD_FORM",
            format!(
                "<form id=\"gold-action-form\" method=\"post\" action=\"/{}/cards/{number}/goldness\">{}<button id=\"gold\" class=\"side-action side-action--gold\" type=\"submit\" aria-label=\"{}\" title=\"{}\" data-method=\"{}\"><span class=\"card-icon card-icon--golden-ticket\" aria-hidden=\"true\"></span></button></form>",
                principal.slug,
                if card.get::<i64, _>("golden") != 0 {
                    "<input type=\"hidden\" name=\"_method\" value=\"delete\">"
                } else {
                    ""
                },
                if card.get::<i64, _>("golden") != 0 {
                    "Remove Golden Ticket"
                } else {
                    "Promote to Golden Ticket"
                },
                if card.get::<i64, _>("golden") != 0 {
                    "Remove Golden Ticket"
                } else {
                    "Promote to Golden Ticket"
                },
                if card.get::<i64, _>("golden") != 0 {
                    "DELETE"
                } else {
                    "POST"
                },
            ),
        ),
        (
            "CLOSURE_FORM",
            if closed {
                String::new()
            } else {
                format!(
                    "<form id=\"closure-action-form\" method=\"post\" action=\"/{}/cards/{number}/closure\"><button id=\"closure\" type=\"submit\" data-method=\"POST\"><span class=\"card-icon card-icon--check\" aria-hidden=\"true\"></span>Mark as Done</button></form>",
                    principal.slug
                )
            },
        ),
        (
            "WATCH_FORM",
            card_controls::button(
                principal.slug,
                number,
                &card_id,
                card_controls::Control::Watch,
                watching != 0,
            ),
        ),
        (
            "PIN_FORM",
            card_controls::button(
                principal.slug,
                number,
                &card_id,
                card_controls::Control::Pin,
                pinned != 0,
            ),
        ),
        ("STAMP", stamp),
        ("CLOSURE_NOTICE", closure_notice),
        ("IMAGE", image),
        ("IMAGE_BUTTON", image_button),
        ("TAG_CONTROL", tag_control),
        (
            "DELETE_ACTION",
            if principal.role == "owner"
                || principal.role == "admin"
                || principal.user_id == card.get::<String, _>("creator_id")
            {
                format!(
                    "<footer class=\"delete-card\"><button type=\"button\" id=\"delete-card-open\" class=\"secondary\">Delete this card</button><dialog id=\"delete-card-dialog\" aria-label=\"Delete this card\"><h3>Delete this card?</h3><p>Are you sure you want to permanently delete this card?</p><div class=\"actions\"><button type=\"button\" id=\"delete-card-cancel\" class=\"secondary\">Cancel</button><form id=\"delete-card-form\" method=\"post\" action=\"/{}/cards/{number}\"><input type=\"hidden\" name=\"_method\" value=\"delete\"><button type=\"submit\">Delete card</button></form></div><p class=\"error\" role=\"alert\" id=\"delete-card-error\"></p></dialog></footer>",
                    principal.slug
                )
            } else {
                String::new()
            },
        ),
        (
            "IMAGE_CLASS",
            if image_url.as_str().is_some() {
                " has-image"
            } else {
                ""
            }
            .to_owned(),
        ),
        (
            "STATE_CLASS",
            if closed {
                " is-closed"
            } else if postponed {
                " is-postponed"
            } else {
                ""
            }
            .to_owned(),
        ),
    ]);
    values.insert("STATUS", String::new());
    values.insert(
        "COMMENT_EDIT_STYLES",
        include_str!("../static/comment_edit.css").into(),
    );
    values.insert(
        "SHELL_STYLES",
        [
            ":root{--color-canvas:#101b20;--color-ink:#f4f5f6}",
            include_str!("../static/shell_footer.css"),
            include_str!("../static/notification_cards.css"),
            include_str!("../static/menu_overlay.css"),
        ]
        .concat(),
    );
    values.insert(
        "SHELL_FOOTER",
        include_str!("../static/home_footer.html")
            .replace("{{ACCOUNT}}", &principal.slug.to_string()),
    );
    values.insert("SHELL_SCRIPT", include_str!("../static/events.js").into());
    values.insert(
        "TAG_PICKER_SCRIPT",
        include_str!("../static/tag_picker_keyboard.js").into(),
    );
    values.insert(
        "ASSIGNMENT_LIMIT_SCRIPT",
        include_str!("../static/assignment_limit.js").into(),
    );
    values.insert(
        "CARD_EDIT_DRAFT_SCRIPT",
        include_str!("../static/card_edit_draft.js").into(),
    );
    Ok(Html(render_template(TEMPLATE, &values)))
}
