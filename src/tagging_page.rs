use super::*;

pub(super) async fn new(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let p = principal(&app, &headers, &account).await?;
    let card = card(&app, &p, &number).await?;
    let card_id: String = card.get("id");
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
    let frame = format!(
        "<turbo-frame id=\"card_{}_tagging\"><div class=\"tag-picker\"><strong class=\"popup__title\">Tag this…</strong><form id=\"tags_form\" action=\"/{account}/cards/{number}/taggings\" method=\"post\"><input id=\"tag-search\" type=\"search\" name=\"tag_title\" placeholder=\"{placeholder}\" aria-label=\"Tag name or filter\" autocomplete=\"off\" autofocus></form><ul class=\"popup__list\">{options}<li class=\"popup__item\"><button type=\"submit\" form=\"tags_form\">Create tag</button></li></ul></div></turbo-frame>",
        escape(&card_id)
    );
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Tag card | Rizzy</title><style>:root{{font:16px/1.5 system-ui,sans-serif;background:#1b1b1b;color:#f5f0e8}}*{{box-sizing:border-box}}body{{margin:0}}main{{max-width:560px;margin:36px auto;padding:24px;border:1px solid #46433d;border-radius:14px;background:#292826}}a{{color:#e7bd61}}strong{{font-size:22px}}input{{width:100%;margin:16px 0;padding:10px;border:1px solid #666158;border-radius:8px;background:#34322f;color:#fff}}ul{{list-style:none;padding:0}}li{{margin:4px 0}}button{{display:flex;width:100%;justify-content:space-between;padding:10px;border:0;border-radius:7px;background:transparent;color:#f5f0e8;text-align:left;font:inherit;cursor:pointer}}button:hover{{background:#46433d}}</style></head><body><main><p><a href=\"/{account}/cards/{number}\">← Back to card</a></p>{frame}</main><script>document.getElementById('tag-search').addEventListener('input',event=>{{const value=event.target.value.toLocaleLowerCase();document.querySelectorAll('.popup__item[data-filter-target]').forEach(item=>item.hidden=!item.textContent.toLocaleLowerCase().includes(value))}})</script></body></html>"
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
