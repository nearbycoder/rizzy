use axum::response::{Html, IntoResponse, Response};
use regex::Regex;
use sqlx::Row;
use std::sync::OnceLock;

use super::{
    App, Principal, Result, SearchQuery, card, db_err, escape, fts_query, mysql_search,
    page_window, search_card_number,
};

fn sqlite_highlighted(value: &str) -> Option<String> {
    const OPEN: char = '\u{e000}';
    const CLOSE: char = '\u{e001}';
    if !value.contains(OPEN) {
        return None;
    }
    let mut html = String::new();
    let mut fragments = value.split(OPEN);
    html.push_str(&escape(fragments.next().unwrap_or_default()));
    for fragment in fragments {
        if let Some((match_text, rest)) = fragment.split_once(CLOSE) {
            html.push_str("<mark class=\"circled-text\"><span></span>");
            html.push_str(&escape(match_text));
            html.push_str("</mark>");
            html.push_str(&escape(rest));
        } else {
            html.push_str(&escape(fragment));
        }
    }
    Some(html)
}

fn excerpt(value: &str) -> String {
    let text = super::rich_text::plain_text(value);
    let mut chars = text.chars();
    let prefix: String = chars.by_ref().take(197).collect();
    if chars.next().is_some() {
        escape(&prefix) + "..."
    } else {
        escape(&text)
    }
}

fn mysql_matches(word: &str, stems: &[String]) -> bool {
    mysql_search::stem_tokens(word)
        .iter()
        .any(|stem| stems.contains(stem))
}

fn mysql_highlighted(value: &str, stems: &[String]) -> String {
    static WORD: OnceLock<Regex> = OnceLock::new();
    let pattern = WORD.get_or_init(|| Regex::new(r"\w+").expect("word regex"));
    let mut result = String::new();
    let mut end = 0;
    for found in pattern.find_iter(value) {
        result.push_str(&escape(&value[end..found.start()]));
        if mysql_matches(found.as_str(), stems) {
            result.push_str("<mark class=\"circled-text\"><span></span>");
            result.push_str(&escape(found.as_str()));
            result.push_str("</mark>");
        } else {
            result.push_str(&escape(found.as_str()));
        }
        end = found.end();
    }
    result.push_str(&escape(&value[end..]));
    result
}

fn mysql_excerpt(value: &str, stems: &[String]) -> String {
    let text = super::rich_text::plain_text(value);
    let words: Vec<&str> = text.split_whitespace().collect();
    if words.len() <= 20 {
        return mysql_highlighted(&text, stems);
    }
    let Some(hit) = words.iter().position(|word| mysql_matches(word, stems)) else {
        return excerpt(value);
    };
    let start = hit.saturating_sub(10);
    let end = (start + 20).min(words.len());
    let mut result = String::new();
    if start > 0 {
        result.push_str("...");
    }
    result.push_str(&mysql_highlighted(&words[start..end].join(" "), stems));
    if end < words.len() {
        result.push_str("...");
    }
    result
}

fn footer(slug: i64) -> String {
    include_str!("../static/home_footer.html").replace("{{ACCOUNT}}", &slug.to_string())
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
    html.push_str(include_str!("../static/shell_footer.css"));
    html.push_str(include_str!("../static/notification_cards.css"));
    html.push_str(include_str!("../static/menu_overlay.css"));
    html.push_str("</style></head><body data-account=\"");
    html.push_str(&p.slug.to_string());
    html.push_str("\"><header class=\"header header--mobile-actions-stack\" id=\"header\"><a href=\"#main\" class=\"header__skip-navigation btn\">Skip to main content</a><a href=\"/");
    html.push_str(&p.slug.to_string());
    html.push_str("/my/menu\" class=\"header__brand\" data-open-menu aria-haspopup=\"dialog\" aria-expanded=\"false\" aria-label=\"Rizzy menu\"><span class=\"header__brand-mark\" aria-hidden=\"true\"></span>Rizzy<span class=\"header__brand-chevron\" aria-hidden=\"true\">⌄</span></a><a href=\"/");
    html.push_str(&p.slug.to_string());
    html.push_str("\" class=\"header__back\">Back to Home</a><h1 class=\"header__title\" data-bridge--title-target=\"header\">");
    html.push_str(&title);
    html.push_str(
        "</h1></header><dialog id=\"menu-dialog\" aria-label=\"Rizzy menu\"><turbo-frame id=\"my_menu\"></turbo-frame></dialog><main id=\"main\"><div class=\"search-perma margin-block-start\"><form class=\"search__form flex align-center justify-center gap-half\" data-controller=\"search-form\" data-action=\"search-form:reset-&gt;bar#reset\" data-bar-target=\"form\" data-turbo-action=\"advance\" data-turbo-frame=\"bar_content\" action=\"/",
    );
    html.push_str(&p.slug.to_string());
    html.push_str("/search\" method=\"get\"><label class=\"font-weight-black txt-nowrap\" for=\"q\">Search Rizzy</label><input id=\"q\" name=\"q\" type=\"search\" class=\"search__input input\" placeholder=\"Find something…\" autocomplete=\"off\" data-search-form-target=\"searchInput\" data-bar-target=\"searchInput\" data-action=\"keydown.enter-&gt;bar#showModalAndSubmit:prevent keydown.esc-&gt;bar#reset\" value=\"");
    html.push_str(&escape(term.unwrap_or("")));
    html.push_str("\" autofocus><button hidden>Search</button><button type=\"button\" class=\"search__reset btn btn--circle borderless\" data-action=\"search-form#clearInput\" aria-label=\"Close search\"><span class=\"icon icon--close\" aria-hidden=\"true\">×</span><span class=\"for-screen-reader\">Close search</span></button></form><turbo-frame id=\"bar_content\">");

    if let Some(term) = term {
        if let Some(number) = search_card_number(term) {
            if card(app, p, &number.to_string()).await.is_ok() {
                html.push_str("<form data-controller=\"auto-submit\" action=\"/");
                html.push_str(&p.slug.to_string());
                html.push_str("/cards/");
                html.push_str(&number.to_string());
                html.push_str("\" method=\"get\"></form><script>document.querySelector('[data-controller=\"auto-submit\"]').requestSubmit()</script></turbo-frame></div></main>");
                html.push_str(&footer(p.slug));
                html.push_str("<script>");
                html.push_str(include_str!("../static/events.js"));
                html.push_str("</script></body></html>");
                return Ok(Html(html).into_response());
            }
        }
    }

    html.push_str("<section class=\"search\"><div class=\"search__results\">");
    let blank_position = html.len();
    html.push_str("<ul class=\"search__list\"><div class=\"filtered_search_results display-contents\" data-controller=\"pagination\" data-pagination-paginate-on-intersection-value=\"true\" id=\"filtered_search_results-pagination-list\"><turbo-frame role=\"presentation\" id=\"filtered_search_results-pagination-contents-");
    html.push_str(&page.to_string());
    html.push_str("\">");
    let mut found = 0;
    let mut next = false;
    if let Some(term) = term {
        let search = if app.mysql {
            mysql_search::query(term, &p.account_id)
        } else {
            fts_query(term)
        };
        if let Some(search) = search {
            let (offset, limit) = page_window(page);
            let rows = if app.mysql {
                let table = mysql_search::table(&p.account_id);
                let statement = format!(
                    r"SELECT CASE WHEN s.searchable_type='Card' THEN 'card' ELSE 'comment' END kind,
                     CASE WHEN s.searchable_type='Comment' THEN s.searchable_id ELSE NULL END hit_id,
                     c.number,c.title,c.description,c.creator_id,u.name creator_name,
                     b.name board_name,m.body comment_body,s.created_at hit_at
                     FROM `{table}` s JOIN cards c ON c.id=s.card_id JOIN boards b ON b.id=c.board_id
                     JOIN users u ON u.id=c.creator_id
                     LEFT JOIN comments m ON m.id=s.searchable_id AND s.searchable_type='Comment'
                     WHERE s.account_id=? AND c.status='published'
                     AND EXISTS(SELECT 1 FROM accesses x WHERE x.board_id=c.board_id AND x.user_id=?)
                     AND MATCH(s.account_key,s.content,s.title) AGAINST(? IN BOOLEAN MODE)
                     ORDER BY s.created_at DESC,s.searchable_id DESC LIMIT ? OFFSET ?"
                );
                sqlx::query(&statement)
                    .bind(&p.account_id)
                    .bind(&p.user_id)
                    .bind(&search)
                    .bind(limit + 1)
                    .bind(offset)
                    .fetch_all(&app.db)
                    .await
                    .map_err(db_err)?
            } else {
                sqlx::query(
                r"WITH hits AS (
                   SELECT 'card' kind,c.id card_id,NULL hit_id,c.created_at hit_at,
                          highlight(card_search,0,char(57344),char(57345)) title_hl,
                          snippet(card_search,1,char(57344),char(57345),'...',20) content_hl
                     FROM card_search JOIN cards c ON c.rowid=card_search.rowid
                    WHERE card_search MATCH ?
                   UNION ALL
                   SELECT 'comment' kind,m.card_id,m.id,m.created_at,NULL,
                          snippet(comment_search,0,char(57344),char(57345),'...',20)
                     FROM comment_search JOIN comments m ON m.rowid=comment_search.rowid
                    WHERE comment_search MATCH ?
                 ), page_hits AS MATERIALIZED (
                   SELECT hits.kind,hits.card_id,hits.hit_id,hits.hit_at,hits.title_hl,hits.content_hl,
                          COALESCE(hits.hit_id,hits.card_id) sort_id
                     FROM hits JOIN cards c ON c.id=hits.card_id
                    WHERE c.account_id=? AND c.status='published'
                      AND EXISTS(SELECT 1 FROM accesses x WHERE x.board_id=c.board_id AND x.user_id=?)
                    ORDER BY hits.hit_at DESC,COALESCE(hits.hit_id,hits.card_id) ASC LIMIT ? OFFSET ?
                 ) SELECT page_hits.kind,page_hits.hit_id,page_hits.hit_at,page_hits.title_hl,page_hits.content_hl,
                          c.number,c.title,c.description,c.creator_id,u.name creator_name,
                          b.name board_name,m.body comment_body
                   FROM page_hits JOIN cards c ON c.id=page_hits.card_id
                   JOIN boards b ON b.id=c.board_id
                   JOIN users u ON u.id=c.creator_id
                   LEFT JOIN comments m ON m.id=page_hits.hit_id
                   ORDER BY page_hits.hit_at DESC,page_hits.sort_id ASC",
            )
            .bind(&search)
            .bind(&search)
            .bind(&p.account_id)
            .bind(&p.user_id)
            .bind(limit + 1)
            .bind(offset)
                .fetch_all(&app.db)
                .await
                .map_err(db_err)?
            };
            next = rows.len() as i64 > limit;
            let mysql_stems = if app.mysql {
                mysql_search::stem_tokens(term)
            } else {
                Vec::new()
            };
            for row in rows.into_iter().take(limit as usize) {
                found += 1;
                let kind: String = row.get("kind");
                let number: i64 = row.get("number");
                let title: String = row.get("title");
                let board: String = row.get("board_name");
                let creator_id: String = row.get("creator_id");
                let creator_name: String = row.get("creator_name");
                let hit_at: String = row.get("hit_at");
                let description: String = row.get("description");
                let comment: Option<String> = row.get("comment_body");
                let hit_id: Option<String> = row.get("hit_id");
                let title_hl: Option<String> = if app.mysql { None } else { row.get("title_hl") };
                let content_hl: Option<String> = if app.mysql {
                    None
                } else {
                    row.get("content_hl")
                };
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
                html.push_str("\" data-turbo-frame=\"_top\" data-action=\"bar#reset\"><div><h3 class=\"search__title txt--medium margin-none\"># ");
                html.push_str(&number.to_string());
                html.push(' ');
                if app.mysql {
                    html.push_str(&mysql_highlighted(&title, &mysql_stems));
                } else if kind == "card" {
                    html.push_str(
                        &title_hl
                            .as_deref()
                            .and_then(sqlite_highlighted)
                            .unwrap_or_else(|| escape(&title)),
                    );
                } else {
                    html.push_str(&escape(&title));
                }
                html.push_str("</h3><div class=\"search__excerpt");
                if kind == "comment" {
                    html.push_str(" search__excerpt--comment");
                }
                html.push_str("\">");
                if kind == "comment" {
                    html.push_str("<span class=\"avatar\" aria-hidden=\"false\" aria-label=\"");
                    html.push_str(&escape(&creator_name));
                    html.push_str("\"><img src=\"/");
                    html.push_str(&p.slug.to_string());
                    html.push_str("/users/");
                    html.push_str(&escape(&creator_id));
                    html.push_str(
                        "/avatar\" aria-hidden=\"true\" width=\"48\" height=\"48\" title=\"",
                    );
                    html.push_str(&escape(&creator_name));
                    html.push_str("\"></span><div>");
                }
                let body = comment.as_deref().unwrap_or(&description);
                if app.mysql {
                    html.push_str(&mysql_excerpt(body, &mysql_stems));
                } else {
                    html.push_str(
                        &content_hl
                            .as_deref()
                            .and_then(sqlite_highlighted)
                            .unwrap_or_else(|| excerpt(body)),
                    );
                }
                if kind == "comment" {
                    html.push_str("</div>");
                }
                html.push_str("</div></div><div class=\"search__board overflow-ellipsis translucent txt-ink\">");
                html.push_str(&escape(&board));
                if let Ok(date) = chrono::DateTime::parse_from_rfc3339(&hit_at) {
                    html.push_str(" · <time datetime=\"");
                    html.push_str(&date.timestamp().to_string());
                    html.push_str("\" data-local-time-target=\"timeordate\" data-action=\"turbo:morph-element-&gt;local-time#refreshTarget\">&nbsp;</time>");
                }
                html.push_str("</div></a></li>");
            }
        }
    }
    if next {
        html.push_str("<a id=\"filtered_search_results-pagination-link-");
        html.push_str(&(page + 1).to_string());
        html.push_str("\" class=\"pagination-link pagination-link--active-when-observed\" aria-label=\"Load page ");
        html.push_str(&(page + 1).to_string());
        html.push_str("\" data-frame=\"filtered_search_results-pagination-contents-");
        html.push_str(&(page + 1).to_string());
        html.push_str("\" data-pagination-target=\"paginationLink\" href=\"/");
        html.push_str(&p.slug.to_string());
        html.push_str("/search?page=");
        html.push_str(&(page + 1).to_string());
        html.push_str("&amp;q=");
        html.push_str(
            &url::form_urlencoded::byte_serialize(term.unwrap_or("").as_bytes())
                .collect::<String>(),
        );
        html.push_str("\">Load more…</a>");
    }
    html.push_str("</turbo-frame></div></ul>");
    if found == 0 && term.is_some() {
        html.insert_str(
            blank_position,
            "<div class=\"search__blank-slate blank-slate\">No matches</div>",
        );
    }
    html.push_str("</div></section></turbo-frame></div></main>");
    html.push_str(&footer(p.slug));
    html.push_str("<script>");
    html.push_str(include_str!("../static/search.js"));
    html.push_str(include_str!("../static/events.js"));
    html.push_str("</script></body></html>");
    Ok(Html(html).into_response())
}
