use axum::response::{Html, IntoResponse, Response};
use regex::{Regex, RegexBuilder};
use sqlx::Row;

use super::{App, Principal, Result, SearchQuery, card, db_err, escape, fts_query, page_window};

fn matcher(query: &str) -> Option<Regex> {
    let mut words: Vec<String> = query
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .filter(|word| !word.is_empty())
        .map(regex::escape)
        .collect();
    words.sort_by_key(|word| std::cmp::Reverse(word.len()));
    words.dedup();
    if words.is_empty() {
        None
    } else {
        RegexBuilder::new(&words.join("|"))
            .case_insensitive(true)
            .build()
            .ok()
    }
}

fn highlighted(value: &str, pattern: Option<&Regex>) -> String {
    let Some(pattern) = pattern else {
        return escape(value);
    };
    let mut result = String::new();
    let mut end = 0;
    for found in pattern.find_iter(value) {
        result.push_str(&escape(&value[end..found.start()]));
        result.push_str("<mark class=\"circled-text\"><span></span>");
        result.push_str(&escape(found.as_str()));
        result.push_str("</mark>");
        end = found.end();
    }
    result.push_str(&escape(&value[end..]));
    result
}

fn excerpt(value: &str, pattern: Option<&Regex>) -> String {
    let text = super::rich_text::plain_text(value);
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.is_empty() {
        return String::new();
    }
    let Some(hit) =
        pattern.and_then(|pattern| words.iter().position(|word| pattern.is_match(word)))
    else {
        let mut chars = text.chars();
        let prefix: String = chars.by_ref().take(197).collect();
        return if chars.next().is_some() {
            escape(&prefix) + "..."
        } else {
            escape(&text)
        };
    };
    let start = hit.saturating_sub(9);
    let end = (hit + 11).min(words.len());
    let mut result = String::new();
    if start > 0 {
        result.push_str("...");
    }
    result.push_str(&highlighted(&words[start..end].join(" "), pattern));
    if end < words.len() {
        result.push_str("...");
    }
    result
}

pub(super) async fn show(app: &App, p: &Principal, query: &SearchQuery) -> Result<Response> {
    let term = query
        .q
        .as_deref()
        .map(str::trim)
        .filter(|term| !term.is_empty());
    let page = query.page.unwrap_or(1).clamp(1, 102);
    let mut html = String::new();
    let title = term
        .map(|term| format!("Search results for &quot;{}&quot;", escape(term)))
        .unwrap_or_else(|| "Search".to_owned());
    html.push_str("<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>");
    html.push_str(&title);
    html.push_str("</title><style>");
    html.push_str(include_str!("../static/search.css"));
    html.push_str("</style></head><body><header class=\"header\"><a href=\"/");
    html.push_str(&p.slug.to_string());
    html.push_str("\">Home</a><h1 class=\"header__title\">");
    html.push_str(&title);
    html.push_str(
        "</h1></header><main class=\"search-perma\"><form class=\"search__form\" action=\"/",
    );
    html.push_str(&p.slug.to_string());
    html.push_str("/search\" method=\"get\"><label for=\"q\">Search Rizzy</label><input id=\"q\" name=\"q\" type=\"search\" class=\"search__input\" placeholder=\"Find something…\" value=\"");
    html.push_str(&escape(term.unwrap_or("")));
    html.push_str("\" autofocus><button type=\"submit\">Search</button></form><turbo-frame id=\"bar_content\">");

    if let Some(term) = term {
        if let Ok(number) = term.parse::<i64>() {
            if card(app, p, &number.to_string()).await.is_ok() {
                html.push_str("<form data-controller=\"auto-submit\" action=\"/");
                html.push_str(&p.slug.to_string());
                html.push_str("/cards/");
                html.push_str(&number.to_string());
                html.push_str("\" method=\"get\"></form><script>document.querySelector('[data-controller=\"auto-submit\"]').requestSubmit()</script></turbo-frame></main></body></html>");
                return Ok(Html(html).into_response());
            }
        }
    }

    html.push_str(
        "<section class=\"search\"><div class=\"search__results\"><ul class=\"search__list\">",
    );
    let mut found = 0;
    let mut next = false;
    if let Some(term) = term {
        if let Some(search) = fts_query(term) {
            let (offset, limit) = page_window(page);
            let rows = sqlx::query(
                r"WITH hits AS (
                   SELECT 'card' kind,c.id card_id,NULL hit_id,c.created_at hit_at FROM cards c
                     WHERE c.rowid IN (SELECT rowid FROM card_search WHERE card_search MATCH ?)
                   UNION ALL
                   SELECT 'comment' kind,m.card_id,m.id,m.created_at FROM comments m
                     WHERE m.rowid IN (SELECT rowid FROM comment_search WHERE comment_search MATCH ?)
                 ), page_hits AS MATERIALIZED (
                   SELECT hits.kind,hits.card_id,hits.hit_id,hits.hit_at
                     FROM hits JOIN cards c ON c.id=hits.card_id
                    WHERE c.account_id=? AND c.status='published'
                      AND EXISTS(SELECT 1 FROM accesses x WHERE x.board_id=c.board_id AND x.user_id=?)
                    ORDER BY hits.hit_at DESC,hits.hit_id DESC LIMIT ? OFFSET ?
                 ) SELECT page_hits.kind,page_hits.hit_id,c.number,c.title,c.description,b.name board_name,m.body comment_body
                   FROM page_hits JOIN cards c ON c.id=page_hits.card_id
                   JOIN boards b ON b.id=c.board_id
                   LEFT JOIN comments m ON m.id=page_hits.hit_id
                   ORDER BY page_hits.hit_at DESC,page_hits.hit_id DESC",
            )
            .bind(&search)
            .bind(&search)
            .bind(&p.account_id)
            .bind(&p.user_id)
            .bind(limit + 1)
            .bind(offset)
            .fetch_all(&app.db)
            .await
            .map_err(db_err)?;
            next = rows.len() as i64 > limit;
            let pattern = matcher(term);
            for row in rows.into_iter().take(limit as usize) {
                found += 1;
                let kind: String = row.get("kind");
                let number: i64 = row.get("number");
                let title: String = row.get("title");
                let board: String = row.get("board_name");
                let description: String = row.get("description");
                let comment: Option<String> = row.get("comment_body");
                let hit_id: Option<String> = row.get("hit_id");
                html.push_str("<li><a class=\"search__result\" href=\"/");
                html.push_str(&p.slug.to_string());
                html.push_str("/cards/");
                html.push_str(&number.to_string());
                if kind == "comment" {
                    if let Some(id) = hit_id {
                        html.push_str("#comment_");
                        html.push_str(&escape(&id));
                    }
                }
                html.push_str("\"><div><h3 class=\"search__title\"># ");
                html.push_str(&number.to_string());
                html.push(' ');
                if kind == "card" {
                    html.push_str(&highlighted(&title, pattern.as_ref()));
                } else {
                    html.push_str(&escape(&title));
                }
                html.push_str("</h3><div class=\"search__excerpt");
                if kind == "comment" {
                    html.push_str(" search__excerpt--comment");
                }
                html.push_str("\">");
                html.push_str(&excerpt(
                    comment.as_deref().unwrap_or(&description),
                    pattern.as_ref(),
                ));
                html.push_str("</div></div><div class=\"search__board\">");
                html.push_str(&escape(&board));
                html.push_str("</div></a></li>");
            }
        }
    }
    html.push_str("</ul>");
    if found == 0 && term.is_some() {
        html.push_str("<div class=\"search__blank-slate blank-slate\">No matches</div>");
    }
    if next {
        html.push_str("<a id=\"filtered_search_results-pagination-link-");
        html.push_str(&(page + 1).to_string());
        html.push_str("\" href=\"/");
        html.push_str(&p.slug.to_string());
        html.push_str("/search?q=");
        html.push_str(
            &url::form_urlencoded::byte_serialize(term.unwrap_or("").as_bytes())
                .collect::<String>(),
        );
        html.push_str("&amp;page=");
        html.push_str(&(page + 1).to_string());
        html.push_str("\">More</a>");
    }
    html.push_str("</div></section></turbo-frame></main></body></html>");
    Ok(Html(html).into_response())
}
