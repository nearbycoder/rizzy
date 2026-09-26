use super::*;

fn menu_link(path: &str, label: &str, kind: &str, key: &str) -> String {
    format!(
        "<li class=\"popup__item\" data-filter-target=\"item\" data-navigable-list-target=\"item\" id=\"filter-{}-{}\"><a class=\"popup__btn btn\" href=\"{}\"><span class=\"overflow-ellipsis\">{}</span></a></li>",
        escape(kind),
        escape(key),
        escape(path),
        escape(label)
    )
}

fn section(title: &str, items: &str) -> String {
    format!(
        "<details class=\"nav__section popup__section\" open><summary class=\"popup__section-title\">{}</summary><ul class=\"popup__list\">{items}</ul></details>",
        escape(title)
    )
}

pub(super) async fn show(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let p = principal(&app, &headers, &account).await?;
    let identity_id: String = sqlx::query_scalar("SELECT identity_id FROM users WHERE id=?")
        .bind(&p.user_id)
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    let tags_statement = if app.mysql {
        "SELECT id,title FROM tags WHERE account_id=? ORDER BY LOWER(title),id"
    } else {
        "SELECT id,title FROM tags WHERE account_id=? ORDER BY title COLLATE NOCASE"
    };
    let users_statement = if app.mysql {
        "SELECT id,name FROM users WHERE account_id=? AND active=1 AND role!='system' ORDER BY LOWER(name),id"
    } else {
        "SELECT id,name FROM users WHERE account_id=? AND active=1 AND role!='system' ORDER BY name COLLATE NOCASE"
    };
    let (boards, tags, users, filters, accounts) = tokio::try_join!(
        sqlx::query("SELECT b.id,b.name FROM boards b JOIN accesses a ON a.board_id=b.id WHERE b.account_id=? AND a.user_id=? ORDER BY a.accessed_at DESC,b.created_at DESC")
            .bind(&p.account_id).bind(&p.user_id).fetch_all(&app.db),
        sqlx::query(tags_statement)
            .bind(&p.account_id).fetch_all(&app.db),
        sqlx::query(users_statement)
            .bind(&p.account_id).fetch_all(&app.db),
        sqlx::query("SELECT id,updated_at FROM filters WHERE account_id=? AND creator_id=? ORDER BY updated_at DESC")
            .bind(&p.account_id).bind(&p.user_id).fetch_all(&app.db),
        sqlx::query("SELECT a.external_account_id,a.name FROM accounts a JOIN users u ON u.account_id=a.id WHERE u.identity_id=? AND u.active=1 AND NOT EXISTS(SELECT 1 FROM account_cancellations c WHERE c.account_id=a.id) ORDER BY a.created_at")
            .bind(&identity_id).fetch_all(&app.db),
    ).map_err(db_err)?;

    let mut body = format!(
        "<turbo-frame id=\"my_menu\" target=\"_top\"><nav class=\"nav\" aria-label=\"Rizzy menu\"><header class=\"nav__header\"><h1>Menu</h1><input type=\"search\" id=\"menu-search\" placeholder=\"Type to jump to a board, person, place, or tag…\" aria-label=\"Search menu\"></header><div class=\"nav__scroll-container\"><div class=\"nav__hotkeys\" role=\"list\">{}{}{}</div>",
        menu_link(&format!("/{}/", p.slug), "Home", "hotkey", "1"),
        menu_link(
            &format!("/{}/cards?assignee_ids%5B%5D={}", p.slug, p.user_id),
            "Assigned to me",
            "hotkey",
            "2"
        ),
        menu_link(
            &format!("/{}/cards?creator_ids%5B%5D={}", p.slug, p.user_id),
            "Added by me",
            "hotkey",
            "3"
        ),
    );
    let mut board_items = menu_link(
        &format!("/{}/boards/new", p.slug),
        "Add a board",
        "board",
        "new",
    );
    for row in boards {
        let id: String = row.get("id");
        board_items.push_str(&menu_link(
            &format!("/{}/boards/{id}", p.slug),
            &row.get::<String, _>("name"),
            "board",
            &id,
        ));
    }
    body.push_str(&section("Boards", &board_items));
    let mut tag_items = String::new();
    for row in tags {
        let id: String = row.get("id");
        tag_items.push_str(&menu_link(
            &format!("/{}/cards?tag_ids%5B%5D={id}", p.slug),
            &format!("#{}", row.get::<String, _>("title")),
            "tag",
            &id,
        ));
    }
    body.push_str(&section("Tags", &tag_items));
    let mut people_items = menu_link(
        &format!("/{}/account/join_code", p.slug),
        "Invite people",
        "person",
        "invite",
    );
    for row in users {
        let id: String = row.get("id");
        people_items.push_str(&menu_link(
            &format!("/{}/users/{id}", p.slug),
            &row.get::<String, _>("name"),
            "person",
            &id,
        ));
    }
    body.push_str(&section("People", &people_items));
    let settings = [
        ("Account Settings", format!("/{}/account/settings", p.slug)),
        ("My Profile", format!("/{}/users/{}", p.slug, p.user_id)),
        ("All notifications", format!("/{}/notifications", p.slug)),
        (
            "Notification Settings",
            format!("/{}/notifications/settings", p.slug),
        ),
    ]
    .into_iter()
    .map(|(label, path)| {
        menu_link(
            &path,
            label,
            "place",
            &label.to_lowercase().replace(' ', "-"),
        )
    })
    .collect::<String>();
    body.push_str(&section("Settings", &settings));
    let shortcuts = [
        ("Golden cards", "indexed_by=golden"),
        ("Stalled cards", "indexed_by=stalled"),
        ("Cards closing soon", "indexed_by=postponing_soon"),
        ("Added today", "creation=today"),
        ("Done today", "closure=today"),
    ]
    .into_iter()
    .map(|(label, query)| {
        menu_link(
            &format!("/{}/cards?{query}", p.slug),
            label,
            "shortcut",
            query,
        )
    })
    .collect::<String>();
    body.push_str(&section("Shortcuts", &shortcuts));
    if accounts.len() > 1 {
        let mut account_items = String::new();
        for row in accounts {
            let slug: i64 = row.get("external_account_id");
            account_items.push_str(&menu_link(
                &format!("/{slug}/landing"),
                &row.get::<String, _>("name"),
                "account",
                &slug.to_string(),
            ));
        }
        body.push_str(&section("Accounts", &account_items));
    }
    body.push_str("</div><footer class=\"nav__footer\">Rizzy</footer></nav><script>document.getElementById('menu-search')?.addEventListener('input',e=>{const q=e.target.value.toLocaleLowerCase();document.querySelectorAll('#my_menu [data-filter-target=item]').forEach(item=>{item.hidden=!item.textContent.toLocaleLowerCase().includes(q)})})</script></turbo-frame>");
    let page = [
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>Menu | Rizzy</title><style>",
        ":root{font:16px/1.5 system-ui,sans-serif;color:#f5f0e8;background:#1b1b1b}*{box-sizing:border-box}body{margin:0}.nav{width:min(480px,100%);min-height:100vh;margin:auto;padding:20px;background:#292826}.nav__header{padding:12px}.nav__header h1{margin:0 0 12px;font-size:26px}.nav__header input{width:100%;padding:12px;border:1px solid #666158;border-radius:9px;background:#34322f;color:#fff}.nav__hotkeys{display:grid;gap:4px;margin-bottom:18px}.nav__section{border-top:1px solid #4a4742;padding:10px 0}.popup__section-title{cursor:pointer;font-weight:700;padding:8px}.popup__list{list-style:none;margin:0;padding:0}.popup__item a{display:block;padding:8px 14px;border-radius:7px;color:inherit;text-decoration:none}.popup__item a:hover,.popup__item a:focus-visible{background:#46433d}.nav__footer{padding:18px;color:#aaa}.popup__item[hidden]{display:none}",
        "</style></head><body>",
        &body,
        "</body></html>",
    ].concat();
    let mut digest = Sha256::new();
    digest.update(page.as_bytes());
    for filter in filters {
        digest.update(filter.get::<String, _>("id").as_bytes());
        digest.update(filter.get::<String, _>("updated_at").as_bytes());
    }
    let etag = format!("W/\"{}\"", hex::encode(digest.finalize()));
    let mut response = if headers
        .get(header::IF_NONE_MATCH)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|value| value.split(',').any(|item| item.trim() == etag))
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
