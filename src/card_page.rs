use axum::response::Html;
use sqlx::{Row, any::AnyRow as DbRow};
use std::collections::HashMap;

use super::{
    App, Principal, Result, assignment_page, board, board_views, comment_edit_page, db_err, escape,
    reaction_page, storage, tagging_page, wrap_html,
};

const TEMPLATE: &str = include_str!("../static/card.html");
const NEW_COMMENT: &str = include_str!("../static/new_comment.html");

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

pub(super) fn edit(principal: &Principal, card: &DbRow) -> Html<String> {
    let card_id: String = card.get("id");
    let board_id: String = card.get("board_id");
    let number: i64 = card.get("number");
    let title: String = card.get("title");
    let description: String = card.get("description");
    let path = format!("/{}/cards/{number}", principal.slug);
    Html(format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Edit card | Rizzy</title><link rel="stylesheet" href="/assets/lexxy/lexxy.css"><link rel="stylesheet" href="/assets/lexxy/rizzy-dark.css"><script type="importmap">{{"imports":{{"@rails/activestorage":"/assets/lexxy/activestorage.esm.js"}}}}</script><script type="module" src="/assets/lexxy/lexxy.js"></script><style>:root{{font:16px/1.5 system-ui,sans-serif;background:#1b1b1b;color:#f5f0e8}}*{{box-sizing:border-box}}body{{margin:0}}main{{max-width:900px;margin:30px auto;padding:24px;border-radius:16px;background:#292826}}a{{color:#e7bd61}}textarea,lexxy-editor{{display:block;width:100%;margin:12px 0;border:1px solid #666158;border-radius:9px;padding:12px;background:#34322f;color:#f5f0e8;font:inherit}}textarea{{min-height:70px}}lexxy-editor{{min-height:180px}}button{{padding:10px 15px;border:0;border-radius:9px;background:#e7bd61;color:#201b12;font:inherit;font-weight:700;cursor:pointer}}</style></head><body><main><p><a href="{path}">← Back to card</a></p><turbo-frame id="edit_card_{card_id}"><turbo-stream action="update" target="card_closure_toggle_card_{card_id}"><template><button type="submit" form="edit_form_card_{card_id}">Save changes</button></template></turbo-stream><form id="edit_form_card_{card_id}" action="{path}" method="post"><input type="hidden" name="_method" value="patch"><h1 class="card__title"><label for="card_title">Name it…</label><textarea id="card_title" name="card[title]" maxlength="255" required autofocus>{title}</textarea></h1><lexxy-editor id="card_description" name="card[description]" value="{description}" placeholder="Add some notes, context, pictures, or video about this…" data-direct-upload-url="/{account}/rails/active_storage/direct_uploads" data-blob-url-template="/{account}/rails/active_storage/blobs/redirect/:signed_id/:filename"><lexxy-prompt trigger="@" src="/{account}/prompts/boards/{board_id}/users" name="mention"></lexxy-prompt><lexxy-prompt trigger="#" src="/{account}/prompts/cards" name="card" insert-editable-text="true" remote-filtering="true" supports-space-in-searches="true"></lexxy-prompt></lexxy-editor><button type="submit">Save changes</button><a href="{path}">Close editor and discard changes</a><p role="alert" id="edit-error"></p></form></turbo-frame></main><script>document.getElementById('edit_form_card_{card_id}').addEventListener('submit',async e=>{{e.preventDefault();const form=e.currentTarget;const editor=document.getElementById('card_description');const response=await fetch(form.action,{{method:'PATCH',headers:{{'Content-Type':'application/json','Accept':'application/json'}},body:JSON.stringify({{card:{{title:document.getElementById('card_title').value,description:editor.value}}}})}});if(response.ok)location.href='{path}';else document.getElementById('edit-error').textContent='Could not save changes'}})</script></body></html>"##,
        path = path,
        card_id = escape(&card_id),
        title = escape(&title),
        description = escape(&description),
        account = principal.slug,
        board_id = escape(&board_id)
    ))
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
    let (_, description_html, _) =
        storage::render_rich_text(app, &principal.account_id, principal.slug, &description).await?;
    let board_name: String = board(app, principal, &board_id).await?.get("name");
    let steps = sqlx::query(
        "SELECT id,content,completed FROM steps WHERE card_id=? ORDER BY created_at,id",
    )
    .bind(&card_id)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let mut step_html = String::new();
    step_html.push_str("<section aria-label=\"Steps\"><ul class=\"steps\">");
    for step in steps {
        let id: String = step.get("id");
        let content: String = step.get("content");
        let checked = if step.get::<i64, _>("completed") != 0 {
            " checked"
        } else {
            ""
        };
        step_html.push_str(&format!(
                "<li class=\"step\" data-step-id=\"{}\"><label class=\"step__checkbox\"><input type=\"checkbox\" data-step-id=\"{}\" aria-label=\"Toggle step completion\"{checked}></label><a class=\"step__content\" data-step-edit href=\"/{}/cards/{}/steps/{}/edit\">{}</a></li>",
                escape(&id),
                escape(&id),
                principal.slug,
                number,
                escape(&id),
                escape(&content)
            ));
    }
    step_html.push_str(&format!("<li id=\"new_step_card_{}\" class=\"step\"><form class=\"steps__form\" id=\"step-form\"><label><span aria-hidden=\"true\">□</span><input id=\"new-step\" name=\"content\" placeholder=\"Add a step…\" aria-label=\"Add a step\"></label></form></li></ul></section>", escape(&card_id)));
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
    let closed = card.get::<i64, _>("closed") != 0;
    let image_button = if image_url.as_str().is_some() {
        "<button class=\"side-action side-action--image\" id=\"remove-image\" type=\"button\" aria-label=\"Remove background image\" title=\"Remove background image\">⊠</button>".to_owned()
    } else if closed {
        String::new()
    } else {
        format!(
            "<form id=\"image-upload-form\" action=\"/{}/cards/{number}\" method=\"post\" enctype=\"multipart/form-data\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><label class=\"side-action side-action--image\" title=\"Add background image\"><span class=\"for-screen-reader\">Add background image</span><svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><rect x=\"2\" y=\"4\" width=\"20\" height=\"16\" rx=\"2\"/><circle cx=\"8\" cy=\"9\" r=\"2\"/><path d=\"m3 18 6-6 4 4 3-3 5 5M18 1v6M15 4h6\"/></svg><input id=\"image-upload\" name=\"card[image]\" type=\"file\" accept=\"image/png,image/jpeg,image/jpg,image/webp\" aria-label=\"Add background image\"></label><noscript><button type=\"submit\">Save background image</button></noscript></form>",
            principal.slug
        )
    };
    let tag_control = tagging_page::tags(app, principal, card).await?;
    let postponed = card.get::<i64, _>("postponed") != 0;
    let column_id: Option<String> = card.get("column_id");
    let columns = sqlx::query("SELECT id,name FROM columns WHERE board_id=? ORDER BY position,id")
        .bind(&board_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let mut stage_rail =
        String::from("<div class=\"stage-rail\" role=\"radiogroup\" aria-label=\"Card stage\">");
    let mut stage = |name: &str, label: &str, current: bool, id: Option<&str>| {
        stage_rail.push_str(&format!("<button type=\"button\" role=\"radio\" aria-checked=\"{}\" class=\"stage-rail__option{}\" data-stage=\"{}\"{}>{}</button>",if current {"true"} else {"false"},if current {" is-current"} else {""},name,id.map(|id|format!(" data-column-id=\"{}\"",escape(id))).unwrap_or_default(),escape(label)));
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
    ]);
    values.insert(
        "STATUS",
        if closed {
            "<span class=\"status\">Done</span>"
        } else {
            ""
        }
        .into(),
    );
    values.insert(
        "CLOSURE_LABEL",
        if closed { "Reopen" } else { "Mark as Done" }.into(),
    );
    values.insert(
        "CLOSURE_METHOD",
        if closed { "DELETE" } else { "POST" }.into(),
    );
    values.insert(
        "WATCH_LABEL",
        if watching != 0 { "Unwatch" } else { "Watch" }.into(),
    );
    values.insert(
        "WATCH_METHOD",
        if watching != 0 { "DELETE" } else { "POST" }.into(),
    );
    values.insert(
        "PIN_LABEL",
        if pinned != 0 { "Unpin" } else { "Pin" }.into(),
    );
    values.insert(
        "PIN_METHOD",
        if pinned != 0 { "DELETE" } else { "POST" }.into(),
    );
    values.insert(
        "GOLD_LABEL",
        if card.get::<i64, _>("golden") != 0 {
            "Remove Golden Ticket"
        } else {
            "Promote to Golden Ticket"
        }
        .into(),
    );
    values.insert(
        "GOLD_METHOD",
        if card.get::<i64, _>("golden") != 0 {
            "DELETE"
        } else {
            "POST"
        }
        .into(),
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
    Ok(Html(render_template(TEMPLATE, &values)))
}
