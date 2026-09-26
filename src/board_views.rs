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
    let title = view.title(column_name.as_deref());
    let mut body = format!(
        "<nav><a href=\"/{account}/public/boards/{key}\">{}</a></nav><h1>{}</h1><section class=\"cards cards--grid\"><div class=\"cards__list\"><ul>",
        escape(&board.get::<String, _>("name")),
        escape(&title)
    );
    for card in &rows {
        let number: i64 = card.get("number");
        let title: String = card.get("title");
        let board_name: String = card.get("board_name");
        let creator: Option<String> = card.get("creator_name");
        let creator = creator.unwrap_or_default();
        let initials = creator
            .split_whitespace()
            .take(2)
            .filter_map(|word| word.chars().next())
            .collect::<String>()
            .to_uppercase();
        let date_label = |raw: String| {
            chrono::DateTime::parse_from_rfc3339(&raw)
                .map(|date| {
                    if date.date_naive() == Utc::now().date_naive() {
                        "TODAY".to_owned()
                    } else {
                        date.format("%b %-d").to_string().to_uppercase()
                    }
                })
                .unwrap_or_default()
        };
        let created = date_label(card.get("created_at"));
        let updated = date_label(card.get("last_active_at"));
        body.push_str(&format!(
            "<li class=\"public-card-preview\"><a class=\"public-card-preview__link\" href=\"/{account}/public/boards/{key}/cards/{number}\"><div class=\"public-card-preview__top\"><span>{number}</span><span>{}</span></div><h3>{}</h3><div class=\"public-card-preview__meta\"><span class=\"public-card-preview__avatar\">{}</span><span>ADDED {created} <span class=\"public-card-preview__separator\">|</span> UPDATED {updated}<br>BY {}</span></div></a></li>",
            escape(&board_name).to_uppercase(),
            escape(&title),
            escape(&initials),
            escape(&creator),
        ));
    }
    if total == 0 {
        body.push_str("<li class=\"blank-slate\">No cards here</li>");
    }
    body.push_str("</ul></div></section>");
    if (page_window(page).0 + rows.len() as i64) < total {
        body.push_str(&format!(
            "<a rel=\"next\" href=\"?page={}\">More cards</a>",
            page + 1
        ));
    }
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
