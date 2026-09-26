use super::*;

const PAGE: &str = include_str!("../static/tagging.html");

pub(super) async fn new(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") || wants_turbo_stream(&headers) {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let p = principal(&app, &headers, &account).await?;
    let card = card(&app, &p, &number).await?;
    let frame = picker(&app, &p, &card).await?;
    let tags = tags(&app, &p, &card).await?;
    let page = card_page::render_template(
        PAGE,
        &HashMap::from([
            ("ACCOUNT", account),
            ("NUMBER", number),
            ("TAGS", tags),
            ("PICKER", frame),
        ]),
    );
    let etag = format!("W/\"{}\"", hex::encode(Sha256::digest(page.as_bytes())));
    let mut response = if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.split(',').any(|part| part.trim() == etag))
    {
        StatusCode::NOT_MODIFIED.into_response()
    } else {
        Html(page).into_response()
    };
    response
        .headers_mut()
        .insert(header::ETAG, HeaderValue::from_str(&etag).unwrap());
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("private, max-age=0, must-revalidate"),
    );
    Ok(response)
}

pub(super) async fn picker(app: &App, p: &Principal, card: &DbRow) -> Result<String> {
    let card_id: String = card.get("id");
    let number: i64 = card.get("number");
    let account = p.slug;
    let tags = sqlx::query("SELECT t.title,EXISTS(SELECT 1 FROM taggings x WHERE x.card_id=? AND x.tag_id=t.id) selected FROM tags t WHERE t.account_id=? ORDER BY selected DESC,lower(t.title),t.id")
        .bind(&card_id).bind(&p.account_id).fetch_all(&app.db).await.map_err(db_err)?;
    let placeholder = if tags.is_empty() {
        "Name this tag…"
    } else {
        "Add a new tag or filter…"
    };
    let mut options = String::new();
    for tag in tags {
        let title: String = tag.get("title");
        let selected = tag.get::<i64, _>("selected") != 0;
        let query = url::form_urlencoded::Serializer::new(String::new())
            .append_pair("tag_title", &title)
            .finish();
        options.push_str(&format!(
            "<li class=\"popup__item\" data-filter-target=\"item\" aria-checked=\"{selected}\"><form method=\"post\" action=\"/{account}/cards/{number}/taggings?{query}\"><button class=\"popup__btn btn\" type=\"submit\">#{title}<span class=\"checked\" aria-hidden=\"true\">{}</span></button></form></li>",
            if selected { "✓" } else { "" },
            title = escape(&title)
        ));
    }
    Ok(format!(
        "<turbo-frame id=\"tagging_card_{}\"><div class=\"tag-picker\"><strong class=\"popup__title\">Tag this…</strong><form id=\"tags_form_card_{}\" action=\"/{account}/cards/{number}/taggings\" method=\"post\"><input id=\"tag-search\" type=\"search\" name=\"tag_title\" placeholder=\"{placeholder}\" aria-label=\"Tag name or filter\" autocomplete=\"off\" autofocus></form><ul class=\"popup__list\">{options}<li class=\"popup__item\"><button type=\"submit\" form=\"tags_form_card_{}\">Create tag</button></li></ul></div></turbo-frame>",
        escape(&card_id),
        escape(&card_id),
        escape(&card_id)
    ))
}

pub(super) async fn tags(app: &App, p: &Principal, card: &DbRow) -> Result<String> {
    let card_id: String = card.get("id");
    let board_id: String = card.get("board_id");
    let number: i64 = card.get("number");
    let rows = sqlx::query("SELECT t.id,t.title FROM tags t JOIN taggings g ON g.tag_id=t.id WHERE g.card_id=? ORDER BY lower(t.title),t.id")
        .bind(&card_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let mut tag_list = String::new();
    for tag in rows {
        let id: String = tag.get("id");
        let title: String = tag.get("title");
        tag_list.push_str(&format!(
            "<a class=\"card-tags__link\" href=\"/{}/cards?board_ids%5B%5D={}&amp;tag_ids%5B%5D={}\">{}</a>",
            p.slug,
            escape(&board_id),
            escape(&id),
            escape(&title)
        ));
    }
    let control = if card.get::<i64, _>("closed") != 0 {
        String::new()
    } else {
        format!(
            "<button class=\"card__tag-picker-button\" type=\"button\" aria-label=\"Add a tag\" title=\"Add a tag\" data-tag-picker-url=\"/{}/cards/{number}/taggings/new\">#</button><dialog class=\"tag-dialog\" aria-label=\"Tag this card\"><button type=\"button\" class=\"tag-dialog__close\" aria-label=\"Close tag picker\">×</button><div class=\"tag-dialog__content\"></div><p class=\"error tag-dialog__error\" role=\"alert\"></p></dialog>",
            p.slug
        )
    };
    Ok(format!(
        "<div id=\"tags_card_{}\" class=\"card__tags\">{control}<div class=\"card-tags__list\">{tag_list}</div></div>",
        escape(&card_id)
    ))
}
