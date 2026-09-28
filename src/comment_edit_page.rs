use super::*;

pub(super) fn comment_frame(
    account: i64,
    number: i64,
    id: &str,
    creator_id: &str,
    creator_name: &str,
    creator_role: &str,
    created_at: &str,
    updated_at: &str,
    content: &str,
    reactions: &str,
    editable: bool,
) -> String {
    let id = escape(id);
    let path = format!("/{account}/cards/{number}/comments/{id}");
    let datetime = chrono::DateTime::parse_from_rfc3339(created_at)
        .map(|date| date.timestamp().to_string())
        .unwrap_or_else(|_| created_at.to_owned());
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
        "<turbo-frame id=\"container_comment_{id}\"{frame_class} data-comment-revision=\"{}\"><article id=\"comment_{id}\" class=\"comment\" data-creator-id=\"{}\">{avatar}<div class=\"comment__content\"><div class=\"comment__author\"><h3><strong><a href=\"/{account}/users/{}\">{}</a></strong> <a class=\"comment__permalink-title\" href=\"{path}\"><time datetime=\"{}\" data-local-time-target=\"agoorweekday\" data-action=\"turbo:morph-element->local-time#refreshTarget\">&nbsp;</time>, <time datetime=\"{}\" data-local-time-target=\"time\" data-action=\"turbo:morph-element->local-time#refreshTarget\">&nbsp;</time></a></h3>{history}{edit}</div><div class=\"comment__body lexxy-content\" data-turbo-permanent>{content}</div>{reactions}</div></article></turbo-frame>",
        escape(updated_at),
        escape(creator_id),
        escape(creator_id),
        escape(creator_name),
        escape(&datetime),
        escape(&datetime),
    )
}

pub(super) async fn rendered_comment_frame(
    app: &App,
    principal: &Principal,
    card: &DbRow,
    comment: &DbRow,
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
        &comment.get::<String, _>("updated_at"),
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
    let principal = principal(&app, &headers, &account).await?;
    let card = card(&app, &principal, &number).await?;
    let comment = comment_row(&app, &card, &id).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    if comment.get::<String, _>("creator_id") != principal.user_id {
        return Err(err(StatusCode::FORBIDDEN, "Comment creator required"));
    }
    let board_id: String = card.get("board_id");
    let body: String = comment.get("body");
    let path = format!("/{account}/cards/{number}/comments/{}", escape(&id));
    let frame = format!(
        r##"<turbo-frame id="container_comment_{id}" class="comment-editor-frame"><div id="comment_{id}" class="comment"><figure class="comment__avatar" aria-hidden="true"><a href="/{account}/users/{user_id}" tabindex="-1"><img src="/{account}/users/{user_id}/avatar" alt="" width="50" height="50"></a></figure><div class="comment__content"><div class="comment__body" data-turbo-permanent><form id="edit_comment_{id}" action="{path}" method="post"><input type="hidden" name="_method" value="patch"><lexxy-editor id="comment_body_{id}" name="comment[body]" value="{body}" required autofocus placeholder="Type your comment…" data-direct-upload-url="/{account}/rails/active_storage/direct_uploads" data-blob-url-template="/{account}/rails/active_storage/blobs/redirect/:signed_id/:filename"><lexxy-prompt trigger="@" src="/{account}/prompts/boards/{board_id}/users" name="mention"></lexxy-prompt><lexxy-prompt trigger="#" src="/{account}/prompts/cards" name="card" insert-editable-text="true" remote-filtering="true" supports-space-in-searches="true"></lexxy-prompt></lexxy-editor><div class="comment-edit-actions"><button type="submit" title="Save changes (Ctrl+Enter)">Save</button><a href="{path}" data-comment-cancel title="Cancel (Esc)">Cancel</a><button type="submit" form="delete_comment_{id}" title="Delete comment" aria-label="Delete"><span class="comment-edit__trash" aria-hidden="true"></span><span class="for-screen-reader">Delete</span></button></div></form><form id="delete_comment_{id}" action="{path}" method="post"><input type="hidden" name="_method" value="delete"></form><p id="comment_error" role="alert"></p></div></div></div></turbo-frame>"##,
        id = escape(&id),
        user_id = escape(&principal.user_id),
        body = escape(&body),
        board_id = escape(&board_id),
    );
    let styles = [
        include_str!("../static/comment_edit.css"),
        include_str!("../static/shell_footer.css"),
        include_str!("../static/notification_cards.css"),
        include_str!("../static/menu_overlay.css"),
    ]
    .concat();
    let values = HashMap::from([
        ("ACCOUNT", principal.slug.to_string()),
        ("FRAME", frame),
        ("STYLES", styles),
        (
            "FOOTER",
            include_str!("../static/home_footer.html")
                .replace("{{ACCOUNT}}", &principal.slug.to_string()),
        ),
        (
            "COMMENT_EDIT_SCRIPT",
            include_str!("../static/comment_edit.js").into(),
        ),
        ("SHELL_SCRIPT", include_str!("../static/events.js").into()),
    ]);
    Ok(Html(card_page::render_template(
        include_str!("../static/comment_edit.html"),
        &values,
    ))
    .into_response())
}
