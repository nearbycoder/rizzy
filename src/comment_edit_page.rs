use super::*;

pub(super) fn comment_frame(
    account: i64,
    number: i64,
    id: &str,
    creator_id: &str,
    creator_name: &str,
    creator_role: &str,
    created_at: &str,
    content: &str,
    reactions: &str,
    editable: bool,
) -> String {
    let id = escape(id);
    let path = format!("/{account}/cards/{number}/comments/{id}");
    let edit = if editable {
        format!(
            "<a class=\"comment__edit\" href=\"{path}/edit\" data-comment-edit aria-label=\"Edit this comment\">Edit</a>"
        )
    } else {
        String::new()
    };
    let system = creator_role == "system";
    let frame_class = if system {
        " class=\"comment-by-system\""
    } else {
        ""
    };
    let avatar = if system {
        String::new()
    } else {
        format!(
            "<figure class=\"comment__avatar\" aria-hidden=\"true\"><a href=\"/{account}/users/{}\" tabindex=\"-1\"><img src=\"/{account}/users/{}/avatar\" alt=\"\" width=\"38\" height=\"38\"></a></figure>",
            escape(creator_id),
            escape(creator_id)
        )
    };
    let history = if system {
        "<button class=\"comment__history\" type=\"button\" data-comment-history>Full history</button>"
    } else {
        ""
    };
    format!(
        "<turbo-frame id=\"container_comment_{id}\"{frame_class}><article id=\"comment_{id}\" class=\"comment\" data-creator-id=\"{}\">{avatar}<div class=\"comment__content\"><div class=\"comment__author\"><h3><strong><a href=\"/{account}/users/{}\">{}</a></strong> <a class=\"comment__permalink-title\" href=\"{path}\"><time datetime=\"{}\">{}</time></a></h3>{history}{edit}</div><div class=\"comment__body lexxy-content\" data-turbo-permanent>{content}</div>{reactions}</div></article></turbo-frame>",
        escape(creator_id),
        escape(creator_id),
        escape(creator_name),
        escape(created_at),
        escape(created_at),
    )
}

pub(super) async fn rendered_comment_frame(
    app: &App,
    principal: &Principal,
    card: &SqliteRow,
    comment: &SqliteRow,
) -> Result<String> {
    let creator_id: String = comment.get("creator_id");
    let creator = sqlx::query("SELECT name,role FROM users WHERE id=? AND account_id=?")
        .bind(&creator_id)
        .bind(&principal.account_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let body: String = comment.get("body");
    let content = if creator.get::<String, _>("role") == "system" {
        wrap_html(&body)
    } else {
        storage::render_rich_text(app, &principal.account_id, principal.slug, &body)
            .await?
            .1
    };
    let reactions =
        reaction_page::reactions_html(app, principal, card, Some(&comment.get::<String, _>("id")))
            .await?;
    Ok(comment_frame(
        principal.slug,
        card.get("number"),
        &comment.get::<String, _>("id"),
        &creator_id,
        &creator.get::<String, _>("name"),
        &creator.get::<String, _>("role"),
        &comment.get::<String, _>("created_at"),
        &content,
        &reactions,
        creator_id == principal.user_id,
    ))
}

pub(super) async fn edit(
    State(app): Shared,
    Path((account, number, id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let principal = principal(&app, &headers, &account).await?;
    let card = card(&app, &principal, &number).await?;
    let comment = comment_row(&app, &card, &id).await?;
    if comment.get::<String, _>("creator_id") != principal.user_id {
        return Err(err(StatusCode::FORBIDDEN, "Comment creator required"));
    }
    let board_id: String = card.get("board_id");
    let body: String = comment.get("body");
    let path = format!("/{account}/cards/{number}/comments/{}", escape(&id));
    let card_path = format!("/{account}/cards/{number}");
    let html = format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Edit comment | Rizzy</title><link rel="stylesheet" href="/assets/lexxy/lexxy.css"><link rel="stylesheet" href="/assets/lexxy/rizzy-dark.css"><script type="importmap">{{"imports":{{"@rails/activestorage":"/assets/lexxy/activestorage.esm.js"}}}}</script><script type="module" src="/assets/lexxy/lexxy.js"></script><style>:root{{font:16px/1.5 system-ui,sans-serif;background:#1b1b1b;color:#f5f0e8}}*{{box-sizing:border-box}}body{{margin:0}}main{{max-width:760px;margin:36px auto;padding:24px;border-radius:14px;background:#292826}}a{{color:#e7bd61}}lexxy-editor{{display:block;min-height:180px;margin:15px 0;border:1px solid #666158;border-radius:9px;background:#34322f;padding:12px}}button,.btn{{display:inline-block;margin-right:8px;padding:9px 15px;border:0;border-radius:8px;background:#e7bd61;color:#201b12;font:inherit;font-weight:700;cursor:pointer;text-decoration:none}}.btn{{background:#46433d;color:#f5f0e8}}.danger{{background:#6a302b;color:#fff}}.actions{{display:flex;align-items:center;gap:8px}}.actions form:last-child{{margin-left:auto}}.comment{{display:flex;gap:12px}}.comment__avatar{{flex:none;margin:8px 0 0}}.comment__avatar img{{display:block;width:38px;height:38px;border-radius:50%;object-fit:cover}}.comment__content{{flex:1;min-width:0}}</style></head><body><main><p><a href="{card_path}">← Back to card</a></p><turbo-frame id="container_comment_{id}"><div id="comment_{id}" class="comment"><figure class="comment__avatar" aria-hidden="true"><a href="/{account}/users/{user_id}" tabindex="-1"><img src="/{account}/users/{user_id}/avatar" alt="" width="38" height="38"></a></figure><div class="comment__content"><div class="comment__body" data-turbo-permanent><form id="edit_comment_{id}" action="{path}" method="post"><input type="hidden" name="_method" value="patch"><lexxy-editor id="comment_body_{id}" name="comment[body]" value="{body}" required autofocus placeholder="Type your comment…" data-direct-upload-url="/{account}/rails/active_storage/direct_uploads" data-blob-url-template="/{account}/rails/active_storage/blobs/redirect/:signed_id/:filename"><lexxy-prompt trigger="@" src="/{account}/prompts/boards/{board_id}/users" name="mention"></lexxy-prompt><lexxy-prompt trigger="#" src="/{account}/prompts/cards" name="card" insert-editable-text="true" remote-filtering="true" supports-space-in-searches="true"></lexxy-prompt></lexxy-editor></form><div class="actions"><button type="submit" form="edit_comment_{id}" title="Save changes (Ctrl+Enter)">Save</button><a class="btn" href="{path}" data-comment-cancel title="Cancel (Esc)">Cancel</a><form id="delete_comment_{id}" action="{path}" method="post"><input type="hidden" name="_method" value="delete"><button class="danger" type="submit">Delete</button></form></div><p id="comment_error" role="alert"></p></div></div></div></turbo-frame></main><script>document.getElementById('edit_comment_{id}').addEventListener('submit',async event=>{{event.preventDefault();const response=await fetch('{path}',{{method:'PATCH',headers:{{'Content-Type':'application/json','Accept':'application/json'}},body:JSON.stringify({{comment:{{body:document.getElementById('comment_body_{id}').value}}}})}});if(response.ok)location.href='{card_path}';else document.getElementById('comment_error').textContent='Could not save changes'}});document.getElementById('delete_comment_{id}').addEventListener('submit',async event=>{{event.preventDefault();if(!confirm('Are you sure you want to delete this comment?'))return;const response=await fetch('{path}',{{method:'DELETE',headers:{{'Accept':'application/json'}}}});if(response.ok)location.href='{card_path}';else document.getElementById('comment_error').textContent='Could not delete comment'}})</script></body></html>"##,
        id = escape(&id),
        user_id = escape(&principal.user_id),
        body = escape(&body),
        board_id = escape(&board_id),
    );
    Ok(Html(html).into_response())
}
