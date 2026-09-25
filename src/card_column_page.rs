use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{Html, IntoResponse, Response},
};
use sqlx::Row;

use super::{Result, Shared, card, db_err, err, escape, principal};

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
    let card_id: String = card.get("id");
    let board_id: String = card.get("board_id");
    let column_id: Option<String> = card.get("column_id");
    let postponed = card.get::<i64, _>("postponed") != 0;
    let closed = card.get::<i64, _>("closed") != 0;
    let columns = sqlx::query(
        "SELECT id,name,color FROM columns WHERE board_id=? AND account_id=? ORDER BY position,id",
    )
    .bind(&board_id)
    .bind(&principal.account_id)
    .fetch_all(&app.db)
    .await
    .map_err(db_err)?;
    let mut html = format!(
        "<turbo-frame id=\"card_{}_columns\"><div class=\"card__stages\" role=\"radiogroup\"><legend class=\"for-screen-reader\">Choose a column for this card</legend>",
        escape(&card_id)
    );
    stage(
        &mut html,
        "Not now",
        "not_now",
        None,
        postponed,
        "var(--color-card-complete)",
    );
    stage(
        &mut html,
        "Maybe?",
        "maybe",
        None,
        !closed && !postponed && column_id.is_none(),
        "var(--color-card-default)",
    );
    for column in columns {
        let id: String = column.get("id");
        let name: String = column.get("name");
        let color: String = column.get("color");
        stage(
            &mut html,
            &name,
            "column",
            Some(&id),
            !closed && !postponed && column_id.as_deref() == Some(id.as_str()),
            &color,
        );
    }
    stage(
        &mut html,
        "Done",
        "done",
        None,
        closed,
        "var(--color-card-complete)",
    );
    html.push_str("</div></turbo-frame>");
    Ok(Html(html).into_response())
}

fn stage(
    html: &mut String,
    label: &str,
    kind: &str,
    column_id: Option<&str>,
    selected: bool,
    color: &str,
) {
    let checked = if selected { "true" } else { "false" };
    let current = if selected {
        " card__column-name--current"
    } else {
        ""
    };
    let disabled = if selected { " disabled" } else { "" };
    let column = column_id
        .map(|id| format!(" data-column-id=\"{}\"", escape(id)))
        .unwrap_or_default();
    let color = safe_color(color);
    html.push_str(&format!(
        "<button type=\"button\" role=\"radio\" aria-checked=\"{checked}\" class=\"card__column-name{current}\" style=\"--column-color: {}\" data-stage=\"{kind}\"{column}{disabled}>{}</button>",
        color, escape(label)
    ));
}

fn safe_color(color: &str) -> &str {
    let hexadecimal = color.strip_prefix('#').is_some_and(|value| {
        matches!(value.len(), 3 | 6 | 8) && value.bytes().all(|ch| ch.is_ascii_hexdigit())
    });
    let variable = color
        .strip_prefix("var(--")
        .and_then(|value| value.strip_suffix(')'))
        .is_some_and(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == b'-')
        });
    if hexadecimal || variable {
        color
    } else {
        "var(--color-card-default)"
    }
}

#[cfg(test)]
mod tests {
    use super::safe_color;

    #[test]
    fn rejects_css_from_column_color() {
        assert_eq!(safe_color("#25a864"), "#25a864");
        assert_eq!(
            safe_color("var(--color-card-default)"),
            "var(--color-card-default)"
        );
        assert_eq!(
            safe_color("red; background: url(https://example.test)"),
            "var(--color-card-default)"
        );
    }
}
