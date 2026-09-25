use super::*;

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
    let meta = meta(&app, &card).await?;
    let page = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Assign card | Rizzy</title><style>:root{{font:16px/1.5 system-ui,sans-serif;background:#1b1b1b;color:#f5f0e8}}*{{box-sizing:border-box}}body{{margin:0}}main{{max-width:560px;margin:36px auto;padding:24px;border:1px solid #46433d;border-radius:14px;background:#292826}}a{{color:#e7bd61}}strong{{font-size:22px}}input{{width:100%;margin:16px 0;padding:10px;border:1px solid #666158;border-radius:8px;background:#34322f;color:#fff}}ul{{list-style:none;padding:0}}li{{margin:4px 0}}button{{display:flex;width:100%;justify-content:space-between;padding:10px;border:0;border-radius:7px;background:transparent;color:#f5f0e8;text-align:left;font:inherit;cursor:pointer}}button:hover{{background:#46433d}}.popup__footer{{color:#afa89d;font-size:13px}}</style></head><body><main><p><a href=\"/{account}/cards/{number}\">← Back to card</a></p>{meta}{frame}<p role=\"alert\" id=\"assignment-error\"></p></main><script>document.querySelector('main').addEventListener('input',event=>{{if(event.target.id!=='user-search')return;const value=event.target.value.toLocaleLowerCase();document.querySelectorAll('.popup__item').forEach(item=>item.hidden=!item.textContent.toLocaleLowerCase().includes(value))}});document.querySelector('main').addEventListener('submit',async event=>{{const form=event.target;if(!form.closest('turbo-frame'))return;event.preventDefault();const button=event.submitter;button.disabled=true;document.getElementById('assignment-error').textContent='';try{{const response=await fetch(form.action,{{method:'POST',headers:{{Accept:'text/vnd.turbo-stream.html'}}}});if(!response.ok)throw Error('Could not update assignment');const update=new DOMParser().parseFromString(await response.text(),'text/html');const stream=update.querySelector('turbo-stream[target]'),replacement=stream?.querySelector('template')?.content.firstElementChild,old=stream&&document.getElementById(stream.getAttribute('target'));if(!old||!replacement)throw Error('Assignment update missing');old.replaceWith(document.importNode(replacement,true));const frame=update.querySelector('turbo-frame[id^=assignment_card_]');if(!frame)throw Error('Assignment picker update missing');document.getElementById(frame.id)?.replaceWith(document.importNode(frame,true))}}catch(error){{document.getElementById('assignment-error').textContent=error.message;button.disabled=false}}}})</script></body></html>"
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

pub(super) async fn picker(app: &App, p: &Principal, card: &SqliteRow) -> Result<String> {
    let account = p.slug;
    let number: i64 = card.get("number");
    let card_id: String = card.get("id");
    let board_id: String = card.get("board_id");
    let users = sqlx::query("SELECT u.id,u.name,EXISTS(SELECT 1 FROM assignments x WHERE x.card_id=? AND x.assignee_id=u.id) assigned FROM users u JOIN accesses a ON a.user_id=u.id AND a.board_id=? WHERE u.account_id=? AND u.active=1 AND u.role!='system' ORDER BY CASE WHEN u.id=? THEN 0 WHEN EXISTS(SELECT 1 FROM assignments x WHERE x.card_id=? AND x.assignee_id=u.id) THEN 1 ELSE 2 END,lower(u.name),u.id")
        .bind(&card_id).bind(&board_id).bind(&p.account_id).bind(&p.user_id).bind(&card_id)
        .fetch_all(&app.db).await.map_err(db_err)?;
    let count = users
        .iter()
        .filter(|user| user.get::<i64, _>("assigned") != 0)
        .count();
    let mut options = String::new();
    for user in users {
        let user_id: String = user.get("id");
        let name: String = user.get("name");
        let label = if user_id == p.user_id { "Me" } else { &name };
        let selected = user.get::<i64, _>("assigned") != 0;
        options.push_str(&format!(
            "<li class=\"popup__item\" data-filter-target=\"item\" aria-checked=\"{}\"><form method=\"post\" action=\"/{account}/cards/{number}/assignments?assignee_id={}\"><button class=\"popup__btn btn\" type=\"submit\">{}<span class=\"checked\" aria-hidden=\"true\">{}</span></button></form></li>",
            selected,
            escape(&user_id),
            escape(label),
            if selected { "✓" } else { "" }
        ));
    }
    Ok(format!(
        "<turbo-frame id=\"assignment_card_{}\"><div class=\"assignment-picker\" data-assignment-limit-count-value=\"{count}\" data-assignment-limit-limit-value=\"100\"><strong class=\"popup__title\">Assign this to…</strong><input type=\"search\" id=\"user-search\" placeholder=\"Filter…\" aria-label=\"Filter people\" autofocus><ul class=\"popup__list\">{options}</ul><div class=\"popup__footer\">Maximum 100 assignees</div></div></turbo-frame>",
        escape(&card_id)
    ))
}

pub(super) async fn meta(app: &App, card: &SqliteRow) -> Result<String> {
    let card_id: String = card.get("id");
    let creator_name: String = sqlx::query_scalar("SELECT name FROM users WHERE id=?")
        .bind(card.get::<String, _>("creator_id"))
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let assignees: Vec<String> = sqlx::query_scalar("SELECT u.name FROM assignments a JOIN users u ON u.id=a.assignee_id WHERE a.card_id=? ORDER BY lower(u.name),u.id")
        .bind(&card_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    let names = if assignees.is_empty() {
        "Not assigned".to_owned()
    } else {
        format!(
            "Assigned to {}",
            assignees
                .iter()
                .map(|name| format!("<strong>{}</strong>", escape(name)))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    Ok(format!(
        "<div class=\"card__meta\" id=\"meta_card_{}\"><span class=\"byline\">By {} · {}</span><span class=\"card__meta-text card__meta-text--assignees\">{names}</span></div>",
        escape(&card_id),
        escape(&creator_name),
        escape(&card.get::<String, _>("created_at"))
    ))
}
