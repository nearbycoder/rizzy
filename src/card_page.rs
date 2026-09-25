use axum::response::Html;
use sqlx::{Row, sqlite::SqliteRow};
use std::collections::HashMap;

use super::{App, Principal, Result, board, db_err, escape, storage, wrap_html};

const TEMPLATE: &str = include_str!("../static/card.html");

pub(super) fn edit(principal: &Principal, card: &SqliteRow) -> Html<String> {
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
    card: &SqliteRow,
) -> Result<Html<String>> {
    let card_id: String = card.get("id");
    let board_id: String = card.get("board_id");
    let number: i64 = card.get("number");
    let title: String = card.get("title");
    let description: String = card.get("description");
    let (_, description_html, _) =
        storage::render_rich_text(app, &principal.account_id, principal.slug, &description).await?;
    let board_name: String = board(app, principal, &board_id).await?.get("name");
    let creator_name: String = sqlx::query_scalar("SELECT name FROM users WHERE id=?")
        .bind(card.get::<String, _>("creator_id"))
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
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
                "<li><label><input type=\"checkbox\" data-step-id=\"{}\"{checked}><span>{}</span></label></li>",
                escape(&id),
                escape(&content)
            ));
    }
    step_html.push_str("</ul><form class=\"steps__form\" id=\"step-form\"><label><span aria-hidden=\"true\">□</span><input id=\"new-step\" name=\"content\" placeholder=\"Add a step…\" aria-label=\"Add a step\"></label></form></section>");
    let comments = sqlx::query("SELECT c.id,c.body,c.created_at,c.creator_id,u.name AS creator_name,u.role AS creator_role FROM comments c JOIN users u ON u.id=c.creator_id WHERE c.card_id=? ORDER BY c.created_at,c.id")
        .bind(&card_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let mut comment_html = String::new();
    if comments.is_empty() {
        comment_html.push_str("<p class=\"empty\">No comments yet.</p>");
    }
    for comment in comments {
        let body: String = comment.get("body");
        let content = if comment.get::<String, _>("creator_role") == "system" {
            wrap_html(&body)
        } else {
            storage::render_rich_text(app, &principal.account_id, principal.slug, &body)
                .await?
                .1
        };
        let comment_id: String = comment.get("id");
        let edit = if comment.get::<String, _>("creator_id") == principal.user_id {
            format!(
                " <a href=\"/{}/cards/{number}/comments/{}/edit\">Edit</a>",
                principal.slug,
                escape(&comment_id)
            )
        } else {
            String::new()
        };
        comment_html.push_str(&format!(
            "<article class=\"comment\"><h3>{} <time>{}</time>{edit} <a href=\"/{}/cards/{number}/comments/{}/reactions/new\">React</a></h3><div class=\"comment__body lexxy-content\">{content}</div></article>",
            escape(&comment.get::<String, _>("creator_name")),
            escape(&comment.get::<String, _>("created_at")),
            principal.slug,
            escape(&comment_id),
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
        "<button class=\"side-action side-action--image\" id=\"remove-image\" type=\"button\" aria-label=\"Remove background image\" title=\"Remove background image\">⊠</button>".to_owned()
    } else {
        "<label class=\"side-action side-action--image\" title=\"Add background image\"><span class=\"for-screen-reader\">Add background image</span><svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><rect x=\"2\" y=\"4\" width=\"20\" height=\"16\" rx=\"2\"/><circle cx=\"8\" cy=\"9\" r=\"2\"/><path d=\"m3 18 6-6 4 4 3-3 5 5M18 1v6M15 4h6\"/></svg><input id=\"image-upload\" type=\"file\" accept=\"image/png,image/jpeg,image/webp\" aria-label=\"Add background image\"></label>".to_owned()
    };
    let closed = card.get::<i64, _>("closed") != 0;
    let tags = sqlx::query("SELECT t.id,t.title FROM tags t JOIN taggings g ON g.tag_id=t.id WHERE g.card_id=? ORDER BY t.title")
        .bind(&card_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let mut tag_list = String::new();
    let mut tag_choices = String::new();
    for tag in tags {
        let id: String = tag.get("id");
        let title: String = tag.get("title");
        tag_list.push_str(&format!("<a class=\"card-tags__link\" href=\"/{}/cards?board_ids%5B%5D={}&amp;tag_ids%5B%5D={}\">{}</a>",principal.slug,escape(&board_id),escape(&id),escape(&title)));
        tag_choices.push_str(&format!(
            "<button type=\"button\" data-tag-title=\"{}\">Remove #{}</button>",
            escape(&title),
            escape(&title)
        ));
    }
    let picker = if closed {
        String::new()
    } else {
        format!(
            "<details><summary aria-label=\"Add a tag\" title=\"Add a tag\"><svg viewBox=\"0 0 24 24\" aria-hidden=\"true\"><path d=\"M3 11V4h7l11 11-7 7L3 11Z\"/><circle cx=\"8\" cy=\"8\" r=\"1\"/></svg></summary><div class=\"card-tags__picker\"><a href=\"/{}/cards/{number}/taggings/new\">Open tag picker</a><form id=\"tag-form\"><label for=\"tag-title\">Tag name</label><input id=\"tag-title\" name=\"tag_title\" placeholder=\"#tag\" autocomplete=\"off\"><button type=\"submit\">Toggle tag</button></form>{tag_choices}</div></details>",
            principal.slug
        )
    };
    let tag_control = format!(
        "<div class=\"card-tags\">{picker}<div class=\"card-tags__list\">{tag_list}</div></div>"
    );
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
        ("BOARD_ID", escape(&board_id)),
        ("BOARD_NAME", escape(&board_name)),
        ("TITLE", escape(&title)),
        ("DESCRIPTION_HTML", description_html),
        ("DESCRIPTION_VALUE", escape(&description)),
        ("CREATOR_NAME", escape(&creator_name)),
        ("CREATED_AT", escape(&card.get::<String, _>("created_at"))),
        ("COMMENTS", comment_html),
        ("STEPS", step_html),
        ("STAGE_RAIL", stage_rail),
        ("IMAGE", image),
        ("IMAGE_BUTTON", image_button),
        ("TAG_CONTROL", tag_control),
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
    Ok(Html(render_template(TEMPLATE, &values)))
}
