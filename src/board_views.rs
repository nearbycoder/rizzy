use super::*;

#[derive(Clone, Copy)]
enum View {
    Stream,
    Closed,
    NotNow,
    Column,
}

impl View {
    fn title(self, column_name: Option<&str>) -> String {
        match self {
            Self::Stream => "Column: Maybe?".into(),
            Self::Closed => "Column: Done".into(),
            Self::NotNow => "Column: Not now".into(),
            Self::Column => format!("Column: {}", column_name.unwrap_or_default()),
        }
    }

    fn scope(self) -> &'static str {
        match self {
            Self::Stream => {
                "c.status='published' AND c.closed=0 AND c.postponed=0 AND c.column_id IS NULL"
            }
            Self::Closed => "c.closed=1",
            Self::NotNow => "c.status='published' AND c.closed=0 AND c.postponed=1",
            Self::Column => {
                "c.status='published' AND c.closed=0 AND c.postponed=0 AND c.column_id=?"
            }
        }
    }

    fn order(self) -> &'static str {
        match self {
            Self::Closed => "cl.created_at DESC,c.id DESC",
            Self::Stream | Self::Column => "c.golden DESC,c.last_active_at DESC,c.id DESC",
            Self::NotNow => "c.last_active_at DESC,c.id DESC",
        }
    }
}

async fn cards_for_view(
    app: &App,
    board_id: &str,
    view: View,
    column_id: Option<&str>,
    page: i64,
    public: bool,
) -> Result<(Vec<SqliteRow>, i64)> {
    let (offset, limit) = page_window(page);
    let published = if public && matches!(view, View::Closed) {
        " AND c.status='published'"
    } else {
        ""
    };
    let joins = if matches!(view, View::Closed) {
        " JOIN closures cl ON cl.card_id=c.id"
    } else {
        ""
    };
    let from = format!(
        " FROM cards c{joins} WHERE c.board_id=? AND {}{published}",
        view.scope()
    );
    let count_sql = format!("SELECT COUNT(*){from}");
    let mut count = sqlx::query_scalar::<_, i64>(&count_sql).bind(board_id);
    if let Some(column_id) = column_id {
        count = count.bind(column_id);
    }
    let total = count.fetch_one(&app.db).await.map_err(db_err)?;
    let select_sql = format!(
        "SELECT c.*,(SELECT name FROM boards WHERE id=c.board_id) AS board_name,(SELECT name FROM columns WHERE id=c.column_id) AS column_name,(SELECT color FROM columns WHERE id=c.column_id) AS column_color,(SELECT name FROM users WHERE id=c.creator_id) AS creator_name{from} ORDER BY {} LIMIT ? OFFSET ?",
        view.order()
    );
    let mut rows = sqlx::query(&select_sql).bind(board_id);
    if let Some(column_id) = column_id {
        rows = rows.bind(column_id);
    }
    let rows = rows
        .bind(limit)
        .bind(offset)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    Ok((rows, total))
}

#[derive(Default)]
struct PublicPreviewMeta {
    tags: Vec<String>,
    assignees: Vec<(String, String)>,
    image: Option<String>,
    closure: Option<(String, String)>,
    not_now: Option<(String, Option<String>)>,
}

async fn public_preview_metadata(
    app: &App,
    cards: &[SqliteRow],
    slug: i64,
) -> Result<HashMap<String, PublicPreviewMeta>> {
    let ids: Vec<String> = cards.iter().map(|card| card.get("id")).collect();
    let mut metadata: HashMap<String, PublicPreviewMeta> = HashMap::new();
    for row in rows_for_ids(
        app,
        "SELECT g.card_id,t.title FROM taggings g JOIN tags t ON t.id=g.tag_id WHERE g.card_id IN (",
        &ids,
    )
    .await?
    {
        metadata
            .entry(row.get("card_id"))
            .or_default()
            .tags
            .push(row.get("title"));
    }
    for row in rows_for_ids(
        app,
        "SELECT a.card_id,u.id AS user_id,u.name FROM assignments a JOIN users u ON u.id=a.assignee_id WHERE a.card_id IN (",
        &ids,
    )
    .await?
    {
        metadata
            .entry(row.get("card_id"))
            .or_default()
            .assignees
            .push((row.get("user_id"), row.get("name")));
    }
    for row in rows_for_ids(
        app,
        "SELECT a.record_id,b.signed_id,b.filename FROM storage_attachments a JOIN storage_blobs b ON b.id=a.blob_id WHERE a.record_type='Card' AND a.name='image' AND a.record_id IN (",
        &ids,
    )
    .await?
    {
        metadata.entry(row.get("record_id")).or_default().image = Some(
            storage::public_image_path(slug, &row.get::<String, _>("signed_id"), &row.get::<String, _>("filename")),
        );
    }
    if cards.iter().any(|card| card.get::<i64, _>("closed") != 0) {
        for row in rows_for_ids(
            app,
            "SELECT cl.card_id,cl.created_at,u.name FROM closures cl JOIN users u ON u.id=cl.user_id WHERE cl.card_id IN (",
            &ids,
        )
        .await?
        {
            metadata.entry(row.get("card_id")).or_default().closure =
                Some((row.get("created_at"), row.get("name")));
        }
    }
    if cards
        .iter()
        .any(|card| card.get::<i64, _>("postponed") != 0)
    {
        for row in rows_for_ids(
            app,
            "SELECT n.card_id,n.created_at,u.name FROM card_not_nows n LEFT JOIN users u ON u.id=n.user_id WHERE n.card_id IN (",
            &ids,
        )
        .await?
        {
            metadata.entry(row.get("card_id")).or_default().not_now =
                Some((row.get("created_at"), row.get("name")));
        }
    }
    for item in metadata.values_mut() {
        item.tags.sort_by_key(|tag| tag.to_lowercase());
        item.assignees.sort_by_key(|(_, name)| name.to_lowercase());
    }
    Ok(metadata)
}

pub(super) fn public_familiar_name(name: &str) -> String {
    let names: Vec<&str> = name.split_whitespace().collect();
    if names.len() <= 1 {
        return name.to_owned();
    }
    let initials = names[1..]
        .iter()
        .filter_map(|part| part.chars().next())
        .map(|initial| format!("{initial}."))
        .collect::<Vec<_>>()
        .join("");
    format!("{}\u{a0}{initials}", names[0])
}

pub(super) fn public_stamp_date(raw: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|value| value.format("%b %d, %Y").to_string())
        .or_else(|_| {
            chrono::NaiveDateTime::parse_from_str(raw, "%Y-%m-%d %H:%M:%S%.f")
                .map(|value| value.format("%b %d, %Y").to_string())
        })
        .unwrap_or_default()
}

fn public_avatar(account: &str, user_id: &str, name: &str) -> String {
    format!(
        "<span class=\"avatar\" aria-label=\"{}\"><img src=\"/{account}/users/{user_id}/avatar\" width=\"48\" height=\"48\" title=\"{}\" aria-hidden=\"true\"></span>",
        escape(name),
        escape(name)
    )
}

fn public_day_label(raw: &str) -> String {
    chrono::DateTime::parse_from_rfc3339(raw)
        .map(|date| {
            if date.date_naive() == Utc::now().date_naive() {
                "Today".to_owned()
            } else {
                date.format("%b %-d").to_string()
            }
        })
        .unwrap_or_default()
}

fn public_html_title(title: &str) -> String {
    static INLINE_CODE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let pattern = INLINE_CODE.get_or_init(|| regex::Regex::new(r"`([^`]+)`").unwrap());
    pattern
        .replace_all(&escape(title), "<code>$1</code>")
        .into_owned()
}

fn render_public_preview(
    account: &str,
    key: &str,
    card: &SqliteRow,
    meta: &PublicPreviewMeta,
    columns: &[(String, String)],
) -> String {
    let id: String = card.get("id");
    let number: i64 = card.get("number");
    let title: String = card.get("title");
    let board_name: String = card.get("board_name");
    let creator_id: String = card.get("creator_id");
    let creator: String = card
        .get::<Option<String>, _>("creator_name")
        .unwrap_or_default();
    let column_id: Option<String> = card.get("column_id");
    let color: Option<String> = card.get("column_color");
    let color = color.as_deref().unwrap_or("var(--color-card-default)");
    let closed = card.get::<i64, _>("closed") != 0;
    let postponed = card.get::<i64, _>("postponed") != 0;
    let golden = card.get::<i64, _>("golden") != 0;
    let class = format!(
        "card public-card-preview{}{}{}",
        if golden { " golden-effect" } else { "" },
        if postponed { " card--postponed" } else { "" },
        if !closed && !postponed {
            " card--active"
        } else {
            ""
        }
    );
    let mut html = format!(
        "<article id=\"article_card_{}\" class=\"{class}\" style=\"--card-color: {}; view-transition-name: article_card_{}\"><div class=\"flex flex-column flex-item-grow max-inline-size\"><header class=\"card__header\"><div class=\"card__board flex align-start\"><span class=\"card__id\"><span class=\"for-screen-reader\">Card number</span>{number}</span><span class=\"card__board-name\"><span class=\"overflow-ellipsis\">{}</span></span></div>",
        escape(&id),
        escape(color),
        escape(&id),
        escape(&board_name)
    );
    if !meta.tags.is_empty() {
        html.push_str("<div class=\"card__tags\"><span class=\"icon translucent\" aria-hidden=\"true\">#</span><div class=\"min-width overflow-ellipsis\">");
        for (index, tag) in meta.tags.iter().enumerate() {
            html.push_str(&format!(
                "<span class=\"card__tag btn btn--plain overflow-ellipsis\">{}{}</span>",
                escape(tag),
                if index + 1 < meta.tags.len() { "," } else { "" }
            ));
        }
        html.push_str("</div></div>");
    }
    html.push_str("</header><div class=\"card__body justify-space-between\"><div class=\"card__content\"><h1 class=\"card__title overflow-line-clamp\">");
    html.push_str(&public_html_title(&title));
    html.push_str("</h1></div>");
    if let Some(selected) = column_id.as_deref() {
        html.push_str(&format!(
            "<div id=\"columns_card_{}\" class=\"card__stages\">",
            escape(&id)
        ));
        for (column_id, name) in columns {
            html.push_str(&format!(
                "<div class=\"card__column-name overflow-ellipsis flex align-center gap-half btn non-clickable no-hover{}\">{}</div>",
                if column_id == selected { " card__column-name--current" } else { "" },
                escape(name)
            ));
        }
        html.push_str("</div>");
    }
    if closed {
        let (date, closer) = meta
            .closure
            .as_ref()
            .map(|(raw, name)| (public_stamp_date(raw), name.as_str()))
            .unwrap_or_default();
        html.push_str(&format!("<div class=\"card__closed\"><span class=\"card__closed-title\" data-text=\"Done\">Done</span><strong class=\"card__closed-date\">{date}</strong><span class=\"card__closed-by-line\">by <span class=\"card__closed-by\">{}</span></span></div>", escape(closer)));
    } else if postponed {
        let (date, by) = meta
            .not_now
            .as_ref()
            .map(|(raw, name)| (public_stamp_date(raw), name.as_deref().unwrap_or("")))
            .unwrap_or_default();
        html.push_str(&format!("<div class=\"card__closed\"><span class=\"card__closed-title\" data-text=\"Not Now\">Not Now</span><strong class=\"card__closed-date\">{date}</strong><span class=\"card__closed-by-line\">by <span class=\"card__closed-by\">{}</span></span></div>", escape(&public_familiar_name(by))));
    }
    html.push_str("</div></div><footer class=\"card__footer\">");
    html.push_str(&format!(
        "<div class=\"card__meta\" id=\"meta_card_{}\"><div class=\"card__meta-avatars card__meta-avatars--author\">{}</div>",
        escape(&id),
        public_avatar(account, &creator_id, &creator)
    ));
    let created: String = card.get("created_at");
    let updated: String = card.get("last_active_at");
    html.push_str(&format!(
        "<span class=\"card__meta-text card__meta-text--added\">Added <time datetime=\"{}\">{}</time></span><span class=\"card__meta-text card__meta-text--author\">By <strong>{}</strong></span><span class=\"card__meta-text card__meta-text--updated overflow-ellipsis\">Updated <time datetime=\"{}\">{}</time></span>",
        escape(&created),
        escape(&public_day_label(&created)),
        escape(&public_familiar_name(&creator)),
        escape(&updated),
        escape(&public_day_label(&updated))
    ));
    html.push_str("<span class=\"card__meta-text card__meta-text--assignees overflow-ellipsis\">");
    if !meta.assignees.is_empty() {
        html.push_str("Assigned to ");
        for (index, (_, name)) in meta.assignees.iter().enumerate() {
            if index > 0 {
                html.push_str(if index + 1 == meta.assignees.len() {
                    " and "
                } else {
                    ", "
                });
            }
            html.push_str(&format!(
                "<strong>{}</strong>",
                escape(&public_familiar_name(name))
            ));
        }
    }
    html.push_str(&format!("</span><div class=\"card__meta-avatars card__meta-avatars--assignees\" id=\"assignees_card_{}\">", escape(&id)));
    for (user_id, name) in &meta.assignees {
        html.push_str(&public_avatar(account, user_id, name));
    }
    html.push_str("</div></div></footer>");
    if let Some(image) = &meta.image {
        html.push_str(&format!(
            "<div class=\"card__background\"><img src=\"{}\" width=\"120\" height=\"120\" alt=\"\"></div>",
            escape(image)
        ));
    }
    let mut link_title = format!("{title} added by {creator} in {board_name}");
    if !meta.assignees.is_empty() {
        link_title.push_str(" assigned to ");
        link_title.push_str(
            &meta
                .assignees
                .iter()
                .map(|(_, name)| name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    html.push_str(&format!(
        "<a class=\"card__link\" href=\"/{account}/public/boards/{key}/cards/{number}\" title=\"{}\"><span class=\"for-screen-reader\">{}</span></a></article>",
        escape(&link_title),
        escape(&title)
    ));
    html
}

async fn private_view(
    app: &App,
    account: &str,
    board_id: &str,
    headers: &HeaderMap,
    page: i64,
    uri: &Uri,
    view: View,
) -> Result<Response> {
    let p = principal(app, headers, account).await?;
    let b = board(app, &p, board_id).await?;
    let (rows, total) = cards_for_view(app, board_id, view, None, page, false).await?;
    if headers.contains_key("x-rizzy-card-fragment") {
        return Ok(Html(cards_page::previews(app, &p, &rows).await?).into_response());
    }
    let wants_json = uri.path().ends_with(".json")
        || headers
            .get(header::ACCEPT)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.contains("json"));
    if wants_json {
        let cards = cards_json_batch(app, &p, &rows, &base(headers)).await?;
        return Ok(paginated(json!(cards), total, page, uri, headers));
    }
    let title = view.title(None);
    let mut body = format!(
        "<nav><a href=\"/{account}/boards/{board_id}\">{}</a></nav><h1>{}</h1><section class=\"cards cards--grid\"><div class=\"cards__list\"><ul>",
        escape(&b.get::<String, _>("name")),
        escape(&title)
    );
    for card in rows {
        let number: i64 = card.get("number");
        body.push_str(&format!(
            "<li><a href=\"/{account}/cards/{number}\">{}</a></li>",
            escape(&card.get::<String, _>("title"))
        ));
    }
    body.push_str("</ul></div></section>");
    Ok(public_page(&title, &body).into_response())
}

pub(super) async fn private_stream(
    State(app): Shared,
    Path((account, board_id)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    private_view(
        &app,
        &account,
        &board_id,
        &headers,
        query.page.unwrap_or(1).max(1),
        &uri,
        View::Stream,
    )
    .await
}

pub(super) async fn private_closed(
    State(app): Shared,
    Path((account, board_id)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    private_view(
        &app,
        &account,
        &board_id,
        &headers,
        query.page.unwrap_or(1).max(1),
        &uri,
        View::Closed,
    )
    .await
}

pub(super) async fn private_not_now(
    State(app): Shared,
    Path((account, board_id)): Path<(String, String)>,
    headers: HeaderMap,
    Query(query): Query<PageQuery>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    private_view(
        &app,
        &account,
        &board_id,
        &headers,
        query.page.unwrap_or(1).max(1),
        &uri,
        View::NotNow,
    )
    .await
}

async fn public_view(
    app: &App,
    account: &str,
    key: &str,
    column_id: Option<&str>,
    page: i64,
    view: View,
) -> Result<Response> {
    let board = published_board(app, account, key).await?;
    let board_id: String = board.get("id");
    let column_name = if let Some(column_id) = column_id {
        Some(
            sqlx::query_scalar::<_, String>("SELECT name FROM columns WHERE id=? AND board_id=?")
                .bind(column_id)
                .bind(&board_id)
                .fetch_optional(&app.db)
                .await
                .map_err(db_err)?
                .ok_or_else(missing)?,
        )
    } else {
        None
    };
    let (rows, total) = cards_for_view(app, &board_id, view, column_id, page, true).await?;
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let metadata = public_preview_metadata(app, &rows, slug).await?;
    let columns: Vec<(String, String)> =
        sqlx::query_as("SELECT id,name FROM columns WHERE board_id=? ORDER BY position,id")
            .bind(&board_id)
            .fetch_all(&app.db)
            .await
            .map_err(db_err)?;
    let title = view.title(column_name.as_deref());
    let (frame_id, column_path) = match view {
        View::Stream => ("stream_column".to_owned(), "stream".to_owned()),
        View::Closed => ("closed_column".to_owned(), "closed".to_owned()),
        View::NotNow => ("not_now_column".to_owned(), "not_now".to_owned()),
        View::Column => {
            let id = column_id.expect("named public column has an id");
            (format!("cards_column_{id}"), id.to_owned())
        }
    };
    let mut body = format!(
        "<nav><a href=\"/{account}/public/boards/{key}\">{}</a></nav><h1>{}</h1><section class=\"cards cards--grid\"><turbo-frame id=\"{frame_id}\"><div class=\"cards__list hide-scrollbar\"><div id=\"{frame_id}-pagination-list\" class=\"{frame_id} display-contents\" data-controller=\"pagination\" data-pagination-paginate-on-intersection-value=\"true\"><turbo-frame id=\"{frame_id}-pagination-contents-{page}\" role=\"presentation\">",
        escape(&board.get::<String, _>("name")),
        escape(&title)
    );
    let empty_meta = PublicPreviewMeta::default();
    for card in &rows {
        let id: String = card.get("id");
        body.push_str(&render_public_preview(
            account,
            key,
            card,
            metadata.get(&id).unwrap_or(&empty_meta),
            &columns,
        ));
    }
    if total == 0 {
        body.push_str("<div class=\"blank-slate\">No cards here</div>");
    }
    if (page_window(page).0 + rows.len() as i64) < total {
        body.push_str(&format!(
            "<a rel=\"next\" id=\"{frame_id}-pagination-link-{}\" class=\"pagination-link pagination-link--active-when-observed\" aria-label=\"Load page {}\" data-frame=\"{frame_id}-pagination-contents-{}\" data-pagination-target=\"paginationLink\" href=\"/{account}/public/boards/{key}/columns/{column_path}?page={}\">Load more…</a>",
            page + 1,
            page + 1,
            page + 1,
            page + 1
        ));
    }
    body.push_str("</turbo-frame></div></div></turbo-frame></section>");
    let mut response = public_page(&title, &body).into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=30"),
    );
    Ok(response)
}

pub(super) async fn public_stream(
    State(app): Shared,
    Path((account, key)): Path<(String, String)>,
    Query(query): Query<PageQuery>,
) -> Result<Response> {
    public_view(
        &app,
        &account,
        &key,
        None,
        query.page.unwrap_or(1).max(1),
        View::Stream,
    )
    .await
}

pub(super) async fn public_closed(
    State(app): Shared,
    Path((account, key)): Path<(String, String)>,
    Query(query): Query<PageQuery>,
) -> Result<Response> {
    public_view(
        &app,
        &account,
        &key,
        None,
        query.page.unwrap_or(1).max(1),
        View::Closed,
    )
    .await
}

pub(super) async fn public_not_now(
    State(app): Shared,
    Path((account, key)): Path<(String, String)>,
    Query(query): Query<PageQuery>,
) -> Result<Response> {
    public_view(
        &app,
        &account,
        &key,
        None,
        query.page.unwrap_or(1).max(1),
        View::NotNow,
    )
    .await
}

pub(super) async fn public_column(
    State(app): Shared,
    Path((account, key, column)): Path<(String, String, String)>,
    Query(query): Query<PageQuery>,
) -> Result<Response> {
    public_view(
        &app,
        &account,
        &key,
        Some(&column),
        query.page.unwrap_or(1).max(1),
        View::Column,
    )
    .await
}
