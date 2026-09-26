use axum::{
    extract::{Path, RawQuery, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{Html, IntoResponse, Response},
};
use chrono::{DateTime, NaiveDate, TimeZone, Timelike, Utc};
use chrono_tz::Tz;
use serde_json::json;
use sqlx::{Any as Db, QueryBuilder, Row, any::AnyRow as DbRow};
use std::{
    sync::{Arc, Weak},
    time::{Duration, Instant},
};

use super::{
    App, EventsHtmlCacheEntry, Principal, Result, Shared, active_user, db_err, err, escape, hash,
    missing, principal,
};

const ACTIONS: &str = "('card_assigned','card_unassigned','card_published','card_closed','card_reopened','card_collection_changed','card_board_changed','card_postponed','card_auto_postponed','card_triaged','card_sent_back_to_triage','comment_created')";

#[derive(Default)]
struct Params {
    day: Option<String>,
    filter_id: Option<String>,
    filter_key: String,
    boards: Vec<String>,
    creators: Vec<String>,
}

fn params(raw: Option<String>) -> Params {
    let mut result = Params::default();
    if let Some(raw) = raw {
        for (key, value) in url::form_urlencoded::parse(raw.as_bytes()) {
            match key.as_ref() {
                "day" => result.day = Some(value.into_owned()),
                "day[]" => result.day = Some("invalid day array".to_owned()),
                "filter_id" if !value.is_empty() => result.filter_id = Some(value.into_owned()),
                "board_ids[]" if !value.is_empty() => result.boards.push(value.into_owned()),
                "creator_ids[]" if !value.is_empty() => result.creators.push(value.into_owned()),
                _ => {}
            }
        }
    }
    result
}

async fn resolved_params(app: &App, p: &Principal, raw: Option<String>) -> Result<Params> {
    let mut result = params(raw.clone());
    let filter = if result.filter_id.is_some() {
        // Day timelines use only the board and creator scopes of a saved card
        // filter. The source ignores other query filters when filter_id is set.
        super::resolved_card_query(app, p, raw).await?
    } else {
        super::parse_card_query(raw)
    };
    result.filter_key = filter_fingerprint(&filter);
    if result.filter_id.is_some() {
        result.boards = filter.board_ids;
        result.creators = filter.creator_ids;
    }
    Ok(result)
}

fn filter_fingerprint(filter: &super::CardQuery) -> String {
    json!({
        "indexed_by": filter.indexed_by.as_deref().unwrap_or("all"),
        "sorted_by": filter.sorted_by.as_deref().unwrap_or("latest"),
        "assignment_status": filter.assignment_status.as_deref().unwrap_or(""),
        "creation": filter.creation.as_deref().unwrap_or(""),
        "closure": filter.closure.as_deref().unwrap_or(""),
        "board_ids": filter.board_ids,
        "creator_ids": filter.creator_ids,
        "card_ids": filter.card_ids,
        "column_ids": filter.column_ids,
        "tag_ids": filter.tag_ids,
        "assignee_ids": filter.assignee_ids,
        "closer_ids": filter.closer_ids,
        "terms": filter.terms,
    })
    .to_string()
}

fn timezone(headers: &HeaderMap) -> Tz {
    headers
        .get(axum::http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookie| {
            cookie.split(';').find_map(|part| {
                let (key, value) = part.trim().split_once('=')?;
                (key == "timezone").then(|| {
                    url::form_urlencoded::parse(format!("value={value}").as_bytes())
                        .next()
                        .map(|(_, zone)| zone.into_owned())
                })
            })
        })
        .flatten()
        .and_then(|name| super::timezone::resolve(&name))
        .unwrap_or(chrono_tz::UTC)
}

fn selected_day(params: &Params, tz: Tz) -> Result<NaiveDate> {
    match params.day.as_deref() {
        None | Some("") => Ok(Utc::now().with_timezone(&tz).date_naive()),
        Some(value) => NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .or_else(|_| {
                DateTime::parse_from_rfc3339(value).map(|time| time.with_timezone(&tz).date_naive())
            })
            .map_err(|_| missing()),
    }
}

fn start_of_day(day: NaiveDate, tz: Tz) -> Result<DateTime<Utc>> {
    // A local midnight can be skipped by a timezone transition. The earliest
    // valid instant on that date is the boundary used by Rails' all_day range.
    let midnight = day.and_hms_opt(0, 0, 0).ok_or_else(missing)?;
    for minute in 0..180 {
        let local = midnight + chrono::Duration::minutes(minute);
        if let Some(time) = tz.from_local_datetime(&local).earliest() {
            return Ok(time.with_timezone(&Utc));
        }
    }
    Err(missing())
}

fn timestamp(time: DateTime<Utc>) -> String {
    time.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

async fn timeline_etag(
    app: &App,
    p: &Principal,
    params: &Params,
    tz: Tz,
    view: &str,
) -> Result<String> {
    let day = selected_day(params, tz)?;
    let start = timestamp(start_of_day(day, tz)?);
    let end = timestamp(start_of_day(day.succ_opt().ok_or_else(missing)?, tz)?);
    let mut query = base_query("SELECT COUNT(*),MAX(e.created_at),MAX(e.id)", p, params);
    query
        .push(" AND e.created_at >= ")
        .push_bind(&start)
        .push(" AND e.created_at < ")
        .push_bind(&end);
    let version: (i64, Option<String>, Option<String>) = query
        .build_query_as()
        .fetch_one(&app.db)
        .await
        .map_err(db_err)?;
    Ok(format!(
        "W/\"{}\"",
        hash(&format!(
            "timeline:{}:{}:{day}:{tz}:{view}:{}:{version:?}",
            p.account_id, p.user_id, params.filter_key
        ))
    ))
}

fn not_modified(headers: &HeaderMap, etag: &str) -> Option<Response> {
    let matches = headers
        .get(header::IF_NONE_MATCH)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| {
            value
                .split(',')
                .any(|candidate| candidate.trim() == etag || candidate.trim() == "*")
        });
    matches.then(|| cache_headers(StatusCode::NOT_MODIFIED.into_response(), etag))
}

fn cache_headers(mut response: Response, etag: &str) -> Response {
    response
        .headers_mut()
        .insert(header::ETAG, HeaderValue::from_str(etag).expect("hex ETag"));
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("max-age=0, private, must-revalidate"),
    );
    response
}

fn base_query<'a>(select: &str, p: &'a Principal, params: &'a Params) -> QueryBuilder<'a, Db> {
    let mut query = QueryBuilder::new(format!(
        "{select} FROM events e JOIN cards c ON c.id=e.card_id JOIN boards b ON b.id=e.board_id JOIN users u ON u.id=e.creator_id LEFT JOIN comments m ON m.id=e.eventable_id AND e.eventable_type='Comment' WHERE e.account_id="
    ));
    query
        .push_bind(&p.account_id)
        .push(" AND EXISTS (SELECT 1 FROM accesses x WHERE x.board_id=b.id AND x.user_id=")
        .push_bind(&p.user_id)
        .push(") AND e.action IN ")
        .push(ACTIONS)
        .push(" AND (e.eventable_type='Card' OR m.id IS NOT NULL)");
    for (field, values) in [
        ("e.board_id", &params.boards),
        ("e.creator_id", &params.creators),
    ] {
        if !values.is_empty() {
            query.push(" AND ").push(field).push(" IN (");
            let mut separated = query.separated(",");
            for value in values {
                separated.push_bind(value);
            }
            separated.push_unseparated(")");
        }
    }
    query
}

fn column_clause(query: &mut QueryBuilder<'_, Db>, column: &str) {
    match column {
        "added" => {
            query.push(" AND e.action IN ('card_published','card_reopened')");
        }
        "closed" => {
            query.push(" AND e.action='card_closed'");
        }
        _ => {
            query.push(" AND e.action NOT IN ('card_published','card_reopened','card_closed')");
        }
    }
}

const EVENT_FIELDS: &str = "SELECT e.id,e.action,e.created_at,e.eventable_type,e.eventable_id,c.id card_id,c.number,c.title,c.description,c.golden,b.name board_name,u.name creator_name,m.body comment_body";

struct Column {
    id: &'static str,
    title: &'static str,
    index: usize,
    count: i64,
    events: Vec<DbRow>,
}

struct Timeline {
    day: NaiveDate,
    next_day: Option<NaiveDate>,
    columns: Vec<Column>,
    tz: Tz,
}

async fn timeline(app: &App, p: &Principal, params: &Params, tz: Tz) -> Result<Timeline> {
    let day = selected_day(params, tz)?;
    let start = timestamp(start_of_day(day, tz)?);
    let end = timestamp(start_of_day(day.succ_opt().ok_or_else(missing)?, tz)?);
    let mut columns: Vec<Column> = [
        ("added", "Added", 1),
        ("updated", "Updated", 2),
        ("closed", "Done", 3),
    ]
    .into_iter()
    .map(|(id, title, index)| Column {
        id,
        title,
        index,
        count: 0,
        events: Vec::new(),
    })
    .collect();
    // A normal day needs only one indexed read for exact counts and cards.
    let mut query = base_query(EVENT_FIELDS, p, params);
    query
        .push(" AND e.created_at >= ")
        .push_bind(&start)
        .push(" AND e.created_at < ")
        .push_bind(&end)
        .push(" ORDER BY e.created_at DESC,e.id DESC LIMIT 301");
    let rows = query.build().fetch_all(&app.db).await.map_err(db_err)?;
    if rows.len() < 301 {
        for row in rows {
            let action: String = row.get("action");
            let bucket = match action.as_str() {
                "card_published" | "card_reopened" => "added",
                "card_closed" => "closed",
                _ => "updated",
            };
            if let Some(column) = columns.iter_mut().find(|column| column.id == bucket) {
                column.count += 1;
                if column.events.len() < 100 {
                    column.events.push(row);
                }
            }
        }
    } else {
        // Beyond 300 events, fetch exact totals and a bounded first page for
        // each column. This keeps a busy account's HTML work predictable.
        for column in &mut columns {
            let mut count = base_query("SELECT COUNT(*)", p, params);
            count
                .push(" AND e.created_at >= ")
                .push_bind(&start)
                .push(" AND e.created_at < ")
                .push_bind(&end);
            column_clause(&mut count, column.id);
            column.count = count
                .build_query_scalar()
                .fetch_one(&app.db)
                .await
                .map_err(db_err)?;
            let mut events = base_query(EVENT_FIELDS, p, params);
            events
                .push(" AND e.created_at >= ")
                .push_bind(&start)
                .push(" AND e.created_at < ")
                .push_bind(&end);
            column_clause(&mut events, column.id);
            events.push(" ORDER BY e.created_at DESC,e.id DESC LIMIT 100");
            column.events = events.build().fetch_all(&app.db).await.map_err(db_err)?;
        }
    }
    let mut previous = base_query("SELECT e.created_at", p, params);
    previous
        .push(" AND e.created_at < ")
        .push_bind(&start)
        .push(" ORDER BY e.created_at DESC,e.id DESC LIMIT 1");
    let next_day = previous
        .build_query_scalar::<String>()
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
        .and_then(|value| DateTime::parse_from_rfc3339(&value).ok())
        .map(|date| date.with_timezone(&tz).date_naive());
    Ok(Timeline {
        day,
        next_day,
        columns,
        tz,
    })
}

fn filter_query(params: &Params) -> String {
    let mut serializer = url::form_urlencoded::Serializer::new(String::new());
    if let Some(filter_id) = &params.filter_id {
        serializer.append_pair("filter_id", filter_id);
        return serializer.finish();
    }
    for board in &params.boards {
        serializer.append_pair("board_ids[]", board);
    }
    for creator in &params.creators {
        serializer.append_pair("creator_ids[]", creator);
    }
    serializer.finish()
}

fn excerpt(text: &str) -> String {
    let plain = super::rich_text::plain_text(text);
    let mut short: String = plain.chars().take(200).collect();
    if plain.chars().count() > 200 {
        short.push('…');
    }
    escape(&short)
}

fn render_event(event: &DbRow, account: i64) -> String {
    let id: String = event.get("id");
    let action: String = event.get("action");
    let number: i64 = event.get("number");
    let title: String = event.get("title");
    let creator: String = event.get("creator_name");
    let board: String = event.get("board_name");
    let card_id: String = event.get("card_id");
    let comment: Option<String> = event.get("comment_body");
    let body: String = if action == "comment_created" {
        comment.as_deref().map(excerpt).unwrap_or_default()
    } else if action == "card_published" {
        excerpt(&event.get::<String, _>("description"))
    } else {
        String::new()
    };
    let verb = match action.as_str() {
        "card_published" => "added",
        "card_closed" => "closed",
        "card_reopened" => "reopened",
        "card_assigned" => "assigned someone to",
        "card_unassigned" => "removed someone from",
        "comment_created" => "commented on",
        "card_postponed" | "card_auto_postponed" => "moved to Not Now",
        "card_sent_back_to_triage" => "sent back to triage",
        "card_triaged" => "triaged",
        "card_board_changed" => "moved",
        "card_collection_changed" => "updated",
        _ => "updated",
    };
    let comment_fragment = if action == "comment_created" {
        format!(
            "#comment_{}",
            escape(&event.get::<String, _>("eventable_id"))
        )
    } else {
        String::new()
    };
    format!(
        "<a id=\"timelined_event_{}\" class=\"event event--{}{}\" href=\"/{account}/cards/{number}{comment_fragment}\" data-card-id=\"{}\"><span class=\"event__board\"><span class=\"event__number\">#{number}</span> {}</span><span class=\"event__creator\">{}</span><strong>{} {}</strong><span class=\"event__excerpt\">{body}</span></a>",
        escape(&id),
        escape(&action),
        if event.get::<i64, _>("golden") != 0 {
            " golden-effect"
        } else {
            ""
        },
        escape(&card_id),
        escape(&board),
        escape(&creator),
        verb,
        escape(&title)
    )
}

fn render_column(column: &Column, day: NaiveDate, tz: Tz, account: i64, params: &Params) -> String {
    let mut html = format!(
        "<div class=\"events__column\"><h3 class=\"events__column-header\"><span>{} {}{}</span>",
        column.title,
        day.format("%b %-d"),
        if column.count > 0 {
            format!(" ({})", column.count)
        } else {
            String::new()
        }
    );
    if column.count > 0 {
        let filter = filter_query(params);
        html.push_str(&format!("<a class=\"events__maximize-button\" href=\"/{account}/events/day_timeline/columns/{}?day={day}{}\" aria-label=\"Expand {} column\">Expand</a>", column.id, if filter.is_empty() { String::new() } else { format!("&{filter}") }, column.title));
    }
    html.push_str("</h3>");
    let mut last_hour = None;
    for event in &column.events {
        let created: String = event.get("created_at");
        let Ok(time) = DateTime::parse_from_rfc3339(&created) else {
            continue;
        };
        let local = time.with_timezone(&tz);
        let hour = local.hour();
        if last_hour != Some(hour) {
            if last_hour.is_some() {
                html.push_str("</div>");
            }
            html.push_str(&format!("<div class=\"events__time-block\" style=\"grid-area: {}/{}\"><time class=\"event__timestamp\" datetime=\"{}\">{}</time>", 25-hour, column.index, escape(&created), local.format("%-I:%M %p")));
            last_hour = Some(hour);
        }
        html.push_str(&render_event(event, account));
    }
    if last_hour.is_some() {
        html.push_str("</div>");
    }
    if column.count > 100 {
        html.push_str(&format!("<div class=\"events__column-footer\">Showing the 100 most recent ({} are hidden)</div>", column.count-100));
    }
    html.push_str("</div>");
    html
}

fn render_day(timeline: &Timeline, account: i64, params: &Params, pagination_path: &str) -> String {
    let mut html = format!(
        "<turbo-frame id=\"day-timeline-pagination-contents-{}\" role=\"presentation\"><section class=\"events__day\">",
        timeline.day
    );
    if timeline.columns.iter().any(|column| column.count > 0) {
        html.push_str("<div class=\"events__columns\">");
        for column in &timeline.columns {
            html.push_str(&render_column(
                column,
                timeline.day,
                timeline.tz,
                account,
                params,
            ));
        }
        html.push_str("</div>");
    }
    if let Some(next_day) = timeline.next_day {
        let gap = (timeline.day - next_day).num_days() - 1;
        if gap > 0 {
            html.push_str(&format!("<div class=\"events__none\"><h2>{} – {}</h2><p>No activity for {gap} days</p></div>", next_day.succ_opt().unwrap_or(next_day).format("%b %-d"), timeline.day.pred_opt().unwrap_or(timeline.day).format("%b %-d")));
        }
    } else {
        html.push_str(&format!(
            "<div class=\"events__none\"><p>{}</p></div>",
            if timeline.columns.iter().any(|column| column.count > 0) {
                "No more activity"
            } else {
                "No activity"
            }
        ));
    }
    html.push_str("</section>");
    if let Some(next_day) = timeline.next_day {
        let filter = filter_query(params);
        html.push_str(&format!("<a class=\"day-timeline-pagination-link\" href=\"/{account}/{pagination_path}?day={next_day}{}\" data-frame=\"day-timeline-pagination-contents-{next_day}\">Load more…</a>", if filter.is_empty() { String::new() } else { format!("&{filter}") }));
    }
    html.push_str("</turbo-frame>");
    html
}

pub(super) async fn index(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let params = resolved_params(&app, &p, raw).await?;
    let tz = timezone(&headers);
    let etag = timeline_etag(&app, &p, &params, tz, "index").await?;
    if let Some(response) = not_modified(&headers, &etag) {
        return Ok(response);
    }
    let cache_key = format!("index:{etag}");
    if let Some(cached) = app.events_html_cache.lock().await.get(&cache_key) {
        if cached.cached_at.elapsed() < Duration::from_secs(1) {
            return Ok(cache_headers(
                Html(cached.html.clone()).into_response(),
                &etag,
            ));
        }
    }
    let build_lock = {
        let mut locks = app.events_html_build_locks.lock().await;
        if locks.len() > 256 {
            locks.retain(|_, lock| lock.strong_count() > 0);
        }
        if let Some(lock) = locks.get(&cache_key).and_then(Weak::upgrade) {
            lock
        } else {
            let lock = Arc::new(tokio::sync::Mutex::new(()));
            locks.insert(cache_key.clone(), Arc::downgrade(&lock));
            lock
        }
    };
    let _build_guard = build_lock.lock_owned().await;
    if let Some(cached) = app.events_html_cache.lock().await.get(&cache_key) {
        if cached.cached_at.elapsed() < Duration::from_secs(1) {
            return Ok(cache_headers(
                Html(cached.html.clone()).into_response(),
                &etag,
            ));
        }
    }
    let timeline = timeline(&app, &p, &params, tz).await?;
    let boards = sqlx::query("SELECT b.id,b.name FROM boards b JOIN accesses x ON x.board_id=b.id WHERE x.user_id=? AND b.account_id=? ORDER BY lower(b.name),b.id")
        .bind(&p.user_id).bind(&p.account_id).fetch_all(&app.db).await.map_err(db_err)?;
    let users = sqlx::query("SELECT id,name FROM users WHERE account_id=? AND active=1 AND role!='system' ORDER BY lower(name),id")
        .bind(&p.account_id).fetch_all(&app.db).await.map_err(db_err)?;
    let mut board_options = String::new();
    for board in &boards {
        let id: String = board.get("id");
        board_options.push_str(&format!(
            "<label class=\"quick-filter__option\"><input type=\"checkbox\" name=\"board_ids[]\" value=\"{}\"{}>{}</label>",
            escape(&id),
            if params.boards.contains(&id) {
                " checked"
            } else {
                ""
            },
            escape(&board.get::<String, _>("name"))
        ));
    }
    let mut user_options = String::new();
    for user in &users {
        let id: String = user.get("id");
        user_options.push_str(&format!(
            "<label class=\"quick-filter__option\"><input type=\"checkbox\" name=\"creator_ids[]\" value=\"{}\"{}>{}</label>",
            escape(&id),
            if params.creators.contains(&id) {
                " checked"
            } else {
                ""
            },
            escape(&user.get::<String, _>("name"))
        ));
    }
    let title = if boards.len() > 1 {
        if params.boards.is_empty() {
            "Activity across"
        } else {
            "Activity in"
        }
    } else {
        "Latest Activity"
    };
    let body = render_day(&timeline, p.slug, &params, "events/days");
    let html = format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"timezone\" content=\"{tz}\"><title>Home | Rizzy</title><style>{}</style></head><body><a class=\"skip\" href=\"#main\">Skip to main content</a><header class=\"header header--events\"><a class=\"brand\" href=\"/{}/my/menu\">Rizzy</a><a class=\"button\" href=\"/\">Boards</a><h1 class=\"header__title\">{title}</h1><a class=\"button\" href=\"/{}/boards/new\">Add a board</a></header><main id=\"main\"><form class=\"events__filters\" action=\"/{}/events\" method=\"get\"><details class=\"quick-filter\"><summary>Board…</summary><div class=\"quick-filter__options\">{board_options}</div></details><details class=\"quick-filter\"><summary>Person…</summary><div class=\"quick-filter__options\">{user_options}</div></details><button class=\"button\" type=\"submit\">Filter</button><a href=\"/{}/events\">Clear</a></form><div id=\"activity\" class=\"events\" data-controller=\"pagination\">{body}</div></main><script>{}</script></body></html>",
        include_str!("../static/events.css"),
        p.slug,
        p.slug,
        p.slug,
        p.slug,
        include_str!("../static/events.js")
    );
    {
        let mut cache = app.events_html_cache.lock().await;
        if cache.len() >= 128 && !cache.contains_key(&cache_key) {
            cache.clear();
        }
        cache.insert(
            cache_key,
            EventsHtmlCacheEntry {
                html: html.clone(),
                cached_at: Instant::now(),
            },
        );
    }
    Ok(cache_headers(Html(html).into_response(), &etag))
}

pub(super) async fn days(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let params = resolved_params(&app, &p, raw).await?;
    let tz = timezone(&headers);
    let etag = timeline_etag(&app, &p, &params, tz, "days").await?;
    if let Some(response) = not_modified(&headers, &etag) {
        return Ok(response);
    }
    let timeline = timeline(&app, &p, &params, tz).await?;
    Ok(cache_headers(
        Html(render_day(&timeline, p.slug, &params, "events/days")).into_response(),
        &etag,
    ))
}

pub(super) async fn user_events(
    State(app): Shared,
    Path((account, user_id)): Path<(String, String)>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let user = active_user(&app, &p, &user_id).await?;
    let mut params = resolved_params(&app, &p, raw.clone()).await?;
    params.filter_id = None;
    params.boards.clear();
    params.creators = vec![user.get("id")];
    params.filter_key = format!("user:{user_id}");
    let etag = timeline_etag(&app, &p, &params, timezone(&headers), "user").await?;
    if let Some(response) = not_modified(&headers, &etag) {
        return Ok(response);
    }
    Ok(cache_headers(
        Html(user_fragment(&app, &p, &user, &headers, raw).await?).into_response(),
        &etag,
    ))
}

pub(super) async fn user_fragment(
    app: &App,
    p: &Principal,
    user: &DbRow,
    headers: &HeaderMap,
    raw: Option<String>,
) -> Result<String> {
    let id: String = user.get("id");
    let mut params = resolved_params(app, p, raw).await?;
    params.filter_id = None;
    params.boards.clear();
    params.creators = vec![id.clone()];
    params.filter_key = format!("user:{id}");
    let timeline = timeline(app, p, &params, timezone(headers)).await?;
    let heading = if p.user_id == id {
        "What have you been up to?".to_owned()
    } else {
        let name: String = user.get("name");
        format!(
            "What has {} been up to?",
            escape(name.split_whitespace().next().unwrap_or(""))
        )
    };
    let path = format!("users/{}/events", escape(&id));
    Ok(format!(
        "<turbo-frame id=\"user_events\"><h1>{heading}</h1><div class=\"events\" id=\"activity\" data-controller=\"pagination\">{}</div></turbo-frame>",
        render_day(&timeline, p.slug, &params, &path)
    ))
}

pub(super) async fn column(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    RawQuery(raw): RawQuery,
) -> Result<Response> {
    if !["added", "updated", "closed"].contains(&id.as_str()) {
        return Err(err(StatusCode::NOT_FOUND, "Not found"));
    }
    let p = principal(&app, &headers, &account).await?;
    let params = resolved_params(&app, &p, raw).await?;
    let tz = timezone(&headers);
    let etag = timeline_etag(&app, &p, &params, tz, "column").await?;
    if let Some(response) = not_modified(&headers, &etag) {
        return Ok(response);
    }
    let timeline = timeline(&app, &p, &params, tz).await?;
    let selected = timeline
        .columns
        .iter()
        .find(|column| column.id == id)
        .ok_or_else(missing)?;
    let mut events = String::new();
    for event in &selected.events {
        events.push_str(&render_event(event, p.slug));
    }
    Ok(cache_headers(Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} | Rizzy</title><style>{}</style></head><body><header class=\"header\"><a href=\"/{}/\">Activity</a><h1>{} {} ({})</h1></header><main id=\"main\" class=\"events--grid\">{events}</main></body></html>",
        selected.title,
        include_str!("../static/events.css"),
        p.slug,
        selected.title,
        timeline.day.format("%b %-d"),
        selected.count
    )).into_response(), &etag))
}
