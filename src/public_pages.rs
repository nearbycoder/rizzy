use axum::{
    extract::{Path, State},
    http::{HeaderMap, HeaderValue, header},
    response::{Html, IntoResponse, Response},
};
use sqlx::Row;
use std::collections::HashMap;

use super::{Result, Shared, base, board_views, db_err, escape, missing, published_board, storage};

fn shell(
    title: &str,
    social_title: &str,
    description: &str,
    image: &str,
    url: &str,
    content: &str,
) -> Html<String> {
    let title = escape(title);
    let social_title = escape(social_title);
    let description = escape(description);
    let image = escape(image);
    let url = escape(url);
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title><meta property=\"og:title\" content=\"{social_title}\"><meta property=\"og:description\" content=\"{description}\"><meta property=\"og:image\" content=\"{image}\"><meta property=\"og:url\" content=\"{url}\"><meta property=\"twitter:title\" content=\"{social_title}\"><meta property=\"twitter:description\" content=\"{description}\"><meta property=\"twitter:image\" content=\"{image}\"><meta property=\"twitter:card\" content=\"summary_large_image\"><style>{}</style></head><body class=\"public\"><div id=\"global-container\"><header class=\"header\" id=\"header\"><a class=\"header__skip-navigation\" href=\"#main\">Skip to main content</a><a class=\"header__logo\" href=\"/\">Rizzy</a></header><main id=\"main\">{content}</main></div><footer id=\"footer\"><p>Rizzy</p></footer></body></html>",
        include_str!("../static/public.css")
    ))
}

fn public_response(page: Html<String>) -> Response {
    let mut response = page.into_response();
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        HeaderValue::from_static("public, max-age=30"),
    );
    response
}

fn column_section(
    id: &str,
    class_name: &str,
    title: &str,
    count: i64,
    color: &str,
    frame_id: &str,
    path: &str,
    expanded: bool,
) -> String {
    let section = escape(id);
    let path = escape(path);
    format!(
        "<section id=\"{section}\" class=\"cards {class_name} {}\" style=\"--card-color:{}\" data-collapsible-columns-target=\"column\"><div class=\"cards__transition-container\"><header class=\"cards__header\"><button class=\"cards__expander\" type=\"button\" aria-controls=\"{section}\" aria-expanded=\"{expanded}\"><span class=\"cards__expander-count\" data-drag-and-drop-counter=\"true\">{}</span><h2 class=\"cards__expander-title\">{}</h2></button><a class=\"cards__maximize-button\" href=\"{path}\" aria-label=\"Maximize column\">▦</a></header><turbo-frame id=\"{}\" src=\"{path}\"></turbo-frame></div></section>",
        if expanded {
            "is-expanded"
        } else {
            "is-collapsed"
        },
        escape(color),
        if count > 99 {
            "99+".to_owned()
        } else {
            count.to_string()
        },
        escape(title),
        escape(frame_id),
    )
}

pub(super) async fn board(
    State(app): Shared,
    Path((account, key)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let board = published_board(&app, &account, &key).await?;
    let board_id: String = board.get("id");
    let name: String = board.get("name");
    let account_name: String = board.get("account_name");
    let account_id: String = board.get("account_id");
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let description: String = board.get("public_description");
    let (description_text, description_html, _) =
        storage::render_rich_text(&app, &account_id, slug, &description).await?;
    let columns =
        sqlx::query("SELECT id,name,color FROM columns WHERE board_id=? ORDER BY position,id")
            .bind(&board_id)
            .fetch_all(&app.db)
            .await
            .map_err(db_err)?;
    let counts = sqlx::query("SELECT column_id,COUNT(*) count FROM cards WHERE board_id=? AND status='published' AND closed=0 AND postponed=0 AND column_id IS NOT NULL GROUP BY column_id")
        .bind(&board_id).fetch_all(&app.db).await.map_err(db_err)?;
    let counts: HashMap<String, i64> = counts
        .into_iter()
        .map(|row| (row.get("column_id"), row.get("count")))
        .collect();
    let (not_now, closed): (i64, i64) = sqlx::query_as("SELECT COUNT(CASE WHEN postponed=1 AND closed=0 THEN 1 END),COUNT(CASE WHEN closed=1 THEN 1 END) FROM cards WHERE board_id=? AND status='published'")
        .bind(&board_id).fetch_one(&app.db).await.map_err(db_err)?;
    let prefix = format!("/{account}/public/boards/{}", escape(&key));
    let mut content = format!(
        "<h1 class=\"header__title divider\"><span class=\"overflow-ellipsis\">{}</span></h1>",
        escape(&name)
    );
    if !description_text.trim().is_empty() {
        content.push_str("<div class=\"card__board-public-description lexxy-content txt-align-center center margin-block-end\">");
        content.push_str(&description_html);
        content.push_str("</div>");
    }
    content.push_str("<turbo-frame id=\"cards_container\"><div class=\"card-columns hide-scrollbar\" data-controller=\"collapsible-columns\"><div class=\"card-columns__left\">");
    content.push_str(&column_section(
        "not-now",
        "cards--on-deck",
        "Not Now",
        not_now,
        "var(--color-card-complete)",
        "not_now_column",
        &format!("{prefix}/columns/not_now"),
        false,
    ));
    content.push_str("</div>");
    // The pinned Fizzy board view currently displays a fixed Maybe count of two.
    content.push_str(&column_section(
        "maybe",
        "cards--maybe",
        "Maybe?",
        2,
        "var(--color-card-default)",
        "stream_column",
        &format!("{prefix}/columns/stream"),
        true,
    ));
    content.push_str("<div class=\"card-columns__right\">");
    for column in columns {
        let id: String = column.get("id");
        let title: String = column.get("name");
        let color: String = column.get("color");
        content.push_str(&column_section(
            &format!("column_{id}"),
            "cards--doing",
            &title,
            *counts.get(&id).unwrap_or(&0),
            &color,
            &format!("cards_column_{id}"),
            &format!("{prefix}/columns/{id}"),
            false,
        ));
    }
    content.push_str(&column_section(
        "closed-cards",
        "cards--closed",
        "Done",
        closed,
        "var(--color-card-complete)",
        "closed_column",
        &format!("{prefix}/columns/closed"),
        false,
    ));
    content.push_str("</div></div></turbo-frame>");
    content.push_str(r#"<script>
      const paginationObserver = new IntersectionObserver(entries => {
        entries.forEach(entry => {
          if (entry.isIntersecting) loadNextPage(entry.target);
        });
      });
      function observeNextPage(frame) {
        const link = frame.querySelector('.pagination-link');
        if (link) paginationObserver.observe(link);
      }
      async function loadNextPage(link) {
        if (link.dataset.loading) return;
        link.dataset.loading = 'true';
        paginationObserver.unobserve(link);
        try {
          const response = await fetch(link.href);
          if (!response.ok) throw new Error('Could not load more cards');
          const document = new DOMParser().parseFromString(await response.text(), 'text/html');
          const nextList = document.querySelector('.cards__list');
          const list = link.closest('.cards__list');
          const newCards = nextList?.querySelectorAll('.public-card-preview');
          if (!list || !newCards) throw new Error('Missing card list');
          link.before(...newCards);
          const nextLink = nextList.querySelector('.pagination-link');
          link.remove();
          if (nextLink) {
            list.querySelector('turbo-frame[id*="-pagination-contents-"]').append(nextLink);
            paginationObserver.observe(nextLink);
          }
        } catch (error) {
          delete link.dataset.loading;
        }
      }
      async function loadPublicColumn(frame) {
        if (!frame || frame.dataset.loaded || frame.dataset.loading) return;
        frame.dataset.loading = 'true';
        try {
          const response = await fetch(frame.getAttribute('src'));
          if (!response.ok) return;
          const document = new DOMParser().parseFromString(await response.text(), 'text/html');
          const list = document.querySelector('.cards__list');
          if (list) {
            frame.replaceChildren(list);
            frame.dataset.loaded = 'true';
            observeNextPage(frame);
          }
        } catch (error) {
          // Keep the column available for another attempt when its request fails.
        } finally {
          delete frame.dataset.loading;
        }
      }
      document.getElementById('cards_container').addEventListener('click', event => {
        const link = event.target.closest('.pagination-link');
        if (!link) return;
        event.preventDefault();
        loadNextPage(link);
      });
      document.querySelectorAll('.cards__expander').forEach(button => button.addEventListener('click', () => {
        const section = button.closest('.cards');
        const expanded = section.classList.toggle('is-expanded');
        section.classList.toggle('is-collapsed', !expanded);
        button.setAttribute('aria-expanded', String(expanded));
        if (expanded) loadPublicColumn(section.querySelector('turbo-frame'));
      }));
      loadPublicColumn(document.getElementById('stream_column'));
    </script>"#);
    let origin = base(&headers);
    let share_url = format!("{origin}{prefix}");
    let page_title = format!("{name} | Rizzy");
    let social_title = format!("{name} | {account_name}");
    Ok(public_response(shell(
        &page_title,
        &social_title,
        &description_text.chars().take(200).collect::<String>(),
        &format!("{origin}/app-icon.png"),
        &share_url,
        &content,
    )))
}

pub(super) async fn card(
    State(app): Shared,
    Path((account, key, number)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let board = published_board(&app, &account, &key).await?;
    let number: i64 = number.parse().map_err(|_| missing())?;
    let card =
        sqlx::query("SELECT * FROM cards WHERE board_id=? AND number=? AND status='published'")
            .bind(board.get::<String, _>("id"))
            .bind(number)
            .fetch_optional(&app.db)
            .await
            .map_err(db_err)?
            .ok_or_else(missing)?;
    let card_id: String = card.get("id");
    let title: String = card.get("title");
    let description: String = card.get("description");
    let board_name: String = board.get("name");
    let account_id: String = board.get("account_id");
    let slug: i64 = account.parse().map_err(|_| missing())?;
    let image_path = storage::public_image_url_for_card(&app, slug, &account_id, &card_id).await?;
    let (description_text, description_html, _) =
        storage::render_rich_text(&app, &account_id, slug, &description).await?;
    let steps =
        sqlx::query("SELECT content,completed FROM steps WHERE card_id=? ORDER BY created_at,id")
            .bind(&card_id)
            .fetch_all(&app.db)
            .await
            .map_err(db_err)?;
    let tags = sqlx::query("SELECT t.title FROM tags t JOIN taggings g ON g.tag_id=t.id WHERE g.card_id=? ORDER BY lower(t.title),t.id")
        .bind(&card_id).fetch_all(&app.db).await.map_err(db_err)?;
    let column: Option<String> = if let Some(id) = card.get::<Option<String>, _>("column_id") {
        sqlx::query_scalar("SELECT name FROM columns WHERE id=?")
            .bind(id)
            .fetch_optional(&app.db)
            .await
            .map_err(db_err)?
    } else {
        None
    };
    let closed = card.get::<i64, _>("closed") != 0;
    let postponed = card.get::<i64, _>("postponed") != 0;
    let state = if closed {
        "Done".to_owned()
    } else if postponed {
        "Not Now".to_owned()
    } else {
        column.unwrap_or_else(|| "Maybe?".to_owned())
    };
    let prefix = format!("/{account}/public/boards/{}", escape(&key));
    let mut content = format!(
        "<nav class=\"header__actions header__actions--start\"><a href=\"{prefix}\">{}</a></nav><section class=\"card-perma card-perma--public\"><div class=\"card-perma__bg\"><article class=\"card{}\"><header class=\"card__header\"><span class=\"card__board\">{}</span><span class=\"card__number\">#{number}</span>",
        escape(&board_name),
        if card.get::<i64, _>("golden") != 0 {
            " card--golden"
        } else {
            ""
        },
        escape(&board_name),
    );
    for tag in tags {
        content.push_str(&format!(
            "<span class=\"card__tag\">{}</span>",
            escape(&tag.get::<String, _>("title"))
        ));
    }
    content.push_str("</header><div class=\"card__body\"><div class=\"card__content\"><h1 class=\"card__title\"><span class=\"card__title-link\">");
    content.push_str(&escape(&title));
    content.push_str("</span></h1><div class=\"card__description lexxy-content\">");
    content.push_str(&description_html);
    content.push_str("</div></div><div class=\"card__stage\">");
    content.push_str(&escape(&state));
    content.push_str("</div>");
    if closed || postponed {
        let sql = if closed {
            "SELECT c.created_at,u.name FROM closures c LEFT JOIN users u ON u.id=c.user_id WHERE c.card_id=?"
        } else {
            "SELECT n.created_at,u.name FROM card_not_nows n LEFT JOIN users u ON u.id=n.user_id WHERE n.card_id=?"
        };
        if let Some(stamp) = sqlx::query(sql)
            .bind(&card_id)
            .fetch_optional(&app.db)
            .await
            .map_err(db_err)?
        {
            let date = board_views::public_stamp_date(&stamp.get::<String, _>("created_at"));
            let by = board_views::public_familiar_name(
                &stamp.get::<Option<String>, _>("name").unwrap_or_default(),
            );
            let label = if closed { "Done" } else { "Not Now" };
            content.push_str(&format!("<div class=\"card__closed\"><span class=\"card__closed-title\" data-text=\"{label}\">{label}</span><strong class=\"card__closed-date\">{date}</strong><span class=\"card__closed-by-line\">by <span class=\"card__closed-by\">{}</span></span></div>", escape(&by)));
        }
    }
    content.push_str("</div><ol class=\"steps txt-small margin-block\">");
    for step in steps {
        content.push_str("<li class=\"step\"><input class=\"checkbox\" type=\"checkbox\" disabled");
        if step.get::<i64, _>("completed") != 0 {
            content.push_str(" checked");
        }
        content.push_str("><span class=\"step__content\">");
        content.push_str(&escape(&step.get::<String, _>("content")));
        content.push_str("</span></li>");
    }
    content.push_str("</ol><footer class=\"card__footer\"><span class=\"card__state\">");
    content.push_str(&escape(&state));
    content.push_str("</span></footer>");
    if let Some(path) = &image_path {
        content.push_str("<img class=\"card__background-image\" alt=\"\" src=\"");
        content.push_str(&escape(path));
        content.push_str("\">");
    }
    content.push_str("</article></div></section>");
    let origin = base(&headers);
    let share_url = format!("{origin}{prefix}/cards/{number}");
    let page_title = format!("{title} | {board_name}");
    Ok(public_response(shell(
        &page_title,
        &page_title,
        &description_text.chars().take(200).collect::<String>(),
        &image_path
            .as_ref()
            .map(|path| format!("{origin}{path}"))
            .unwrap_or_else(|| format!("{origin}/app-icon.png")),
        &share_url,
        &content,
    )))
}
