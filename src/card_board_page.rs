use super::*;

pub(super) async fn edit(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let principal = principal(&app, &headers, &account).await?;
    let card = card(&app, &principal, &number).await?;
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let boards = sqlx::query("SELECT b.id,b.name FROM boards b JOIN accesses a ON a.board_id=b.id WHERE b.account_id=? AND a.user_id=? ORDER BY a.accessed_at DESC,b.created_at DESC,b.id")
        .bind(&principal.account_id).bind(&principal.user_id)
        .fetch_all(&app.db).await.map_err(db_err)?;
    let current: String = card.get("board_id");
    let mut options = String::new();
    for board in boards {
        let id: String = board.get("id");
        let name: String = board.get("name");
        let selected = if id == current { "true" } else { "false" };
        options.push_str(&format!("<li class=\"popup__item\" data-filter-target=\"item\" data-navigable-list-target=\"item\" aria-checked=\"{selected}\"><form class=\"max-width flex-item-grow\" method=\"post\" action=\"/{}/cards/{}/board?board_id={}\"><input type=\"hidden\" name=\"_method\" value=\"patch\"><button class=\"popup__btn btn\" type=\"submit\"><span class=\"overflow-ellipsis flex-item-grow\">{}</span><span class=\"checked flex-item-no-shrink flex-item-justify-end\" aria-hidden=\"true\">✓</span></button></form></li>",principal.slug,escape(&number),escape(&id),escape(&name)));
    }
    let frame = format!(
        "<turbo-frame id=\"board_picker\"><div class=\"max-width full-width board-picker\" data-controller=\"filter navigable-list\" data-navigable-list-focus-on-selection-value=\"false\" data-navigable-list-actionable-items-value=\"true\"><h1>Move this card to…</h1><input type=\"search\" id=\"board-search\" name=\"search\" class=\"input input--transparent txt-small margin-block-half font-weight-normal\" placeholder=\"Filter…\" aria-label=\"Filter boards\" autocomplete=\"off\" autocorrect=\"off\" data-filter-target=\"input\" autofocus><ul class=\"popup__list margin-block-start-half margin-none-block-end\" data-filter-target=\"list\">{options}</ul></div></turbo-frame>"
    );
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Move card | Rizzy</title><style>:root{{font:16px/1.5 system-ui,sans-serif;color:#f5f0e8;background:#1b1b1b}}*{{box-sizing:border-box}}body{{margin:0}}main{{width:min(540px,calc(100% - 32px));margin:36px auto;padding:24px;background:#292826;border:1px solid #46433d;border-radius:14px}}a{{color:#e7bd61}}h1{{font-size:22px}}input{{width:100%;padding:10px;background:#34322f;border:1px solid #666158;border-radius:8px;color:#fff}}ul{{list-style:none;padding:0}}li{{margin:4px 0}}.checked{{visibility:hidden}}li[aria-checked=true] .checked{{visibility:visible}}button{{width:100%;display:flex;justify-content:space-between;padding:10px;border:0;border-radius:7px;background:transparent;color:#f5f0e8;text-align:left;font:inherit;cursor:pointer}}button:hover{{background:#46433d}}</style></head><body><main><p><a href=\"/{}/cards/{}\">← Back to card</a></p>{frame}</main><script>document.getElementById('board-search')?.addEventListener('input',event=>{{const value=event.target.value.toLocaleLowerCase();document.querySelectorAll('.popup__item').forEach(item=>item.hidden=!item.textContent.toLocaleLowerCase().includes(value))}})</script></body></html>",
        principal.slug,
        escape(&number)
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
