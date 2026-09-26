use super::*;
use chrono::TimeZone;
use crc32fast::Hasher;

struct MailNotice {
    id: String,
    board_id: String,
    board: String,
    card_id: String,
    number: i64,
    title: String,
    creator_id: String,
    creator: String,
    avatar: bool,
    kind: String,
    detail: String,
}

fn initials(name: &str) -> String {
    name.split_whitespace()
        .filter_map(|part| part.chars().next())
        .collect::<String>()
        .to_uppercase()
}

fn avatar_color(id: &str) -> &'static str {
    const COLORS: [&str; 18] = [
        "#AF2E1B", "#CC6324", "#3B4B59", "#BFA07A", "#ED8008", "#ED3F1C", "#BF1B1B", "#736B1E",
        "#D07B53", "#736356", "#AD1D1D", "#BF7C2A", "#C09C6F", "#698F9C", "#7C956B", "#5D618F",
        "#3B3633", "#67695E",
    ];
    let mut hasher = Hasher::new();
    hasher.update(id.as_bytes());
    COLORS[hasher.finalize() as usize % COLORS.len()]
}

fn truncate_words(text: &str, length: usize) -> String {
    if text.chars().count() <= length {
        return text.to_owned();
    }
    let mut prefix = text
        .chars()
        .take(length.saturating_sub(3))
        .collect::<String>();
    if let Some(space) = prefix.rfind(char::is_whitespace) {
        prefix.truncate(space);
    }
    format!("{}...", prefix.trim_end())
}

fn card_html_title(title: &str) -> String {
    let escaped = escape(title);
    static INLINE_CODE: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    INLINE_CODE
        .get_or_init(|| regex::Regex::new(r"`([^`]+)`").expect("valid inline code regex"))
        .replace_all(&escaped, "<code>$1</code>")
        .into_owned()
}

fn to_sentence(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [name] => name.clone(),
        [first, second] => format!("{first} and {second}"),
        _ => format!(
            "{}, and {}",
            names[..names.len() - 1].join(", "),
            names.last().unwrap()
        ),
    }
}

async fn mail_notices(app: &App, user_id: &str, start: &str, end: &str) -> Result<Vec<MailNotice>> {
    let rows = sqlx::query("SELECT n.id,n.source_id,n.source_type,c.id card_id,c.number,c.title,c.description,b.id board_id,b.name board_name,creator.id creator_id,creator.name creator_name,EXISTS(SELECT 1 FROM assignments aa WHERE aa.card_id=c.id AND aa.assignee_id=creator.id) creator_assigned,EXISTS(SELECT 1 FROM storage_attachments a WHERE a.record_type='User' AND a.name='avatar' AND a.record_id=creator.id) creator_avatar,e.action event_action,e.particulars event_particulars,e.eventable_id,m.source_type mention_source_type,m.source_id mention_source_id,comment.body comment_body FROM notifications n JOIN cards c ON c.id=n.card_id JOIN boards b ON b.id=c.board_id JOIN users creator ON creator.id=n.creator_id LEFT JOIN events e ON n.source_type='Event' AND e.id=n.source_id LEFT JOIN mentions m ON n.source_type='Mention' AND m.id=n.source_id LEFT JOIN comments comment ON comment.id=CASE WHEN n.source_type='Event' AND e.eventable_type='Comment' THEN e.eventable_id WHEN n.source_type='Mention' AND m.source_type='Comment' THEN m.source_id END WHERE n.user_id=? AND n.read_at IS NULL AND n.updated_at>=? AND n.updated_at<=? AND ((n.source_type='Event' AND e.id IS NOT NULL) OR (n.source_type='Mention' AND m.id IS NOT NULL)) ORDER BY lower(b.name),b.id,c.number,n.updated_at")
        .bind(user_id).bind(start).bind(end).fetch_all(&app.db).await.map_err(db_err)?;
    let mut notices = Vec::with_capacity(rows.len());
    for row in rows {
        let creator: String = row.get("creator_name");
        let kind: String = row.get("source_type");
        let detail = if kind == "Mention" {
            let content: String = if row
                .get::<Option<String>, _>("mention_source_type")
                .as_deref()
                == Some("Comment")
            {
                row.get::<Option<String>, _>("comment_body")
                    .unwrap_or_default()
            } else {
                row.get("description")
            };
            truncate_words(&rich_text::plain_text(&content), 250)
        } else {
            let action: String = row
                .get::<Option<String>, _>("event_action")
                .unwrap_or_default();
            let particulars: Value = serde_json::from_str(
                &row.get::<Option<String>, _>("event_particulars")
                    .unwrap_or_default(),
            )
            .unwrap_or_default();
            match action.as_str() {
                "card_assigned" => {
                    let ids = particulars["assignee_ids"].as_array();
                    let mut names = Vec::new();
                    for id in ids.into_iter().flatten().filter_map(Value::as_str) {
                        if let Some(name) =
                            sqlx::query_scalar::<_, String>("SELECT name FROM users WHERE id=?")
                                .bind(id)
                                .fetch_optional(&app.db)
                                .await
                                .map_err(db_err)?
                        {
                            names.push(name);
                        }
                    }
                    format!(
                        "Assigned to {}",
                        if names.is_empty() {
                            "self".into()
                        } else {
                            to_sentence(&names)
                        }
                    )
                }
                "card_unassigned" => format!("Unassigned by {creator}"),
                "card_published" if row.get::<i64, _>("creator_assigned") != 0 => {
                    "Assigned to self".into()
                }
                "card_published" => format!("Added by {creator}"),
                "card_closed" => format!("Moved to Done by {creator}"),
                "card_reopened" => format!("Reopened by {creator}"),
                "card_postponed" => format!("Moved to Not Now by {creator}"),
                "card_auto_postponed" => "Moved to Not Now due to inactivity".into(),
                "card_title_changed" => format!("Renamed by {creator}"),
                "card_board_changed" => format!("Moved by {creator}"),
                "card_triaged" => format!(
                    "Moved to {} by {creator}",
                    particulars["particulars"]["column"].as_str().unwrap_or("")
                ),
                "card_sent_back_to_triage" => format!("Moved back to Maybe? by {creator}"),
                "comment_created" => truncate_words(
                    &rich_text::plain_text(
                        &row.get::<Option<String>, _>("comment_body")
                            .unwrap_or_default(),
                    ),
                    200,
                ),
                _ => creator.clone(),
            }
        };
        notices.push(MailNotice {
            id: row.get("id"),
            board_id: row.get("board_id"),
            board: row.get("board_name"),
            card_id: row.get("card_id"),
            number: row.get("number"),
            title: row.get("title"),
            creator_id: row.get("creator_id"),
            creator,
            avatar: row.get::<i64, _>("creator_avatar") != 0,
            kind,
            detail,
        });
    }
    Ok(notices)
}

pub(super) async fn queue(
    tx: &mut sqlx::Transaction<'_, Db>,
    user_id: &str,
    timestamp: &str,
) -> Result<()> {
    let Some(user) = sqlx::query("SELECT u.account_id,u.role,u.active,u.verified_at,COALESCE(s.bundle_email_frequency,'every_few_hours') frequency FROM users u LEFT JOIN notification_settings s ON s.user_id=u.id WHERE u.id=? AND NOT EXISTS(SELECT 1 FROM account_cancellations c WHERE c.account_id=u.account_id)")
        .bind(user_id).fetch_optional(&mut **tx).await.map_err(db_err)? else {
        return Ok(());
    };
    let frequency: String = user.get("frequency");
    if frequency == "never"
        || user.get::<String, _>("role") == "system"
        || user.get::<i64, _>("active") == 0
        || user.get::<Option<String>, _>("verified_at").is_none()
    {
        return Ok(());
    }
    let existing: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM notification_bundles WHERE user_id=? AND status=0 AND starts_at<=? AND ends_at>?)")
        .bind(user_id).bind(timestamp).bind(timestamp)
        .fetch_one(&mut **tx).await.map_err(db_err)?;
    if existing != 0 {
        return Ok(());
    }
    let earlier = sqlx::query("SELECT id FROM notification_bundles WHERE user_id=? AND status=0 AND starts_at>? ORDER BY starts_at LIMIT 1")
        .bind(user_id).bind(timestamp).fetch_optional(&mut **tx).await.map_err(db_err)?;
    if let Some(earlier) = earlier {
        sqlx::query("UPDATE notification_bundles SET starts_at=?,updated_at=? WHERE id=?")
            .bind(timestamp)
            .bind(now())
            .bind(earlier.get::<String, _>("id"))
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        return Ok(());
    }
    let start = chrono::DateTime::parse_from_rfc3339(timestamp)
        .map_err(|_| bad("Invalid notification timestamp"))?;
    let period = match frequency.as_str() {
        "every_few_hours" => chrono::Duration::hours(4),
        "weekly" => chrono::Duration::weeks(1),
        _ => chrono::Duration::days(1),
    };
    let ends_at = (start + period).to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let timestamp_now = now();
    sqlx::query("INSERT INTO notification_bundles(id,account_id,user_id,starts_at,ends_at,status,created_at,updated_at) VALUES(?,?,?,?,?,0,?,?)")
        .bind(id()).bind(user.get::<String,_>("account_id")).bind(user_id)
        .bind(timestamp).bind(ends_at).bind(&timestamp_now).bind(&timestamp_now)
        .execute(&mut **tx).await.map_err(db_err)?;
    Ok(())
}

fn display_time(timestamp: &str, timezone: Option<&str>) -> String {
    let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(timestamp) else {
        return timestamp.to_owned();
    };
    let zone = timezone
        .and_then(super::timezone::resolve)
        .unwrap_or(chrono_tz::UTC);
    let local = zone.from_utc_datetime(&parsed.naive_utc());
    local.format("%-I%P on %A, %B %-d").to_string()
}

#[cfg(test)]
mod tests {
    use super::{MailNotice, display_time, render_mail};

    #[test]
    fn formats_bundle_start_in_rails_madrid_timezone() {
        assert_eq!(
            display_time("2025-01-15T14:30:00.000Z", Some("Madrid")),
            "3pm on Wednesday, January 15"
        );
    }

    #[test]
    fn multipart_body_groups_boards_and_cards_and_escapes_user_content() {
        let notice =
            |id: &str, board_id: &str, board: &str, card_id: &str, number, kind: &str| MailNotice {
                id: id.into(),
                board_id: board_id.into(),
                board: board.into(),
                card_id: card_id.into(),
                number,
                title: "Fix the `bug` & <script>".into(),
                creator_id: "author".into(),
                creator: "Ada Lovelace".into(),
                avatar: false,
                kind: kind.into(),
                detail: "Safe <message>".into(),
            };
        let notices = [
            notice("n1", "a", "Alpha", "c1", 1, "Event"),
            notice("n2", "a", "Alpha", "c1", 1, "Mention"),
            notice("n3", "a", "Alpha", "c2", 2, "Event"),
            notice("n4", "b", "Beta", "c3", 3, "Event"),
        ];
        let (plain, html) = render_mail(
            &notices,
            "3pm on Wednesday, January 15",
            "Team",
            true,
            "https://example.test",
            7,
            "https://example.test/settings",
            "https://example.test/unsubscribe",
        );
        assert!(plain.contains("You have 4 new notifications in Team."));
        assert_eq!(plain.matches("Alpha\n-----").count(), 1);
        assert_eq!(html.matches("class=\"notification__board\"").count(), 2);
        assert_eq!(html.matches("class=\"card__title\"").count(), 3);
        assert!(html.contains("Fix the <code>bug</code> &amp; &lt;script&gt;"));
        assert!(html.contains("Ada mentioned you:"));
        assert!(!html.contains("<script>"));
    }
}

fn render_mail(
    notices: &[MailNotice],
    started: &str,
    account_name: &str,
    multiple_accounts: bool,
    host: &str,
    slug: i64,
    settings: &str,
    unsubscribe_page: &str,
) -> (String, String) {
    let count = notices.len();
    let plural = if count == 1 { "" } else { "s" };
    let account_suffix = if multiple_accounts {
        format!(" in {account_name}")
    } else {
        String::new()
    };
    let notifications_url = format!("{host}/{slug}/notifications");
    let mut plain = format!(
        "Notifications since {started}\nYou have {count} new notification{plural}{account_suffix}.\n\n"
    );
    let mut html = format!(
        "<tr><td><h1 class=\"title\">Notifications since {}</h1><p class=\"subtitle margin-block-end-double\">You have <a href=\"{}\">{count} new notification{plural}</a>{}.</p>",
        escape(started),
        escape(&notifications_url),
        escape(&account_suffix)
    );
    let mut last_board = String::new();
    let mut last_card = String::new();
    for notice in notices {
        if notice.board_id != last_board {
            if !last_board.is_empty() {
                plain.push('\n');
            }
            plain.push_str(&format!(
                "{}\n{}\n\n",
                notice.board,
                "-".repeat(notice.board.chars().count())
            ));
            html.push_str(&format!("<hr class=\"notification__board-separator\"><h2 class=\"notification__board\"><a href=\"{host}/{slug}/boards/{}\">{}</a></h2>", escape(&notice.board_id), escape(&notice.board)));
            last_board = notice.board_id.clone();
            last_card.clear();
        }
        if notice.card_id != last_card {
            plain.push_str(&format!("#{} {}\n", notice.number, notice.title));
            html.push_str(&format!(
                "<a class=\"card__title\" href=\"{host}/{slug}/cards/{}\">#{} {}</a>",
                notice.number,
                notice.number,
                card_html_title(&notice.title)
            ));
            last_card = notice.card_id.clone();
        }
        let notice_url = format!("{host}/{slug}/notifications/{}", notice.id);
        let avatar = if notice.avatar {
            format!(
                "<img class=\"avatar\" alt=\"{}\" src=\"{host}/{slug}/users/{}/avatar\" width=\"48\" height=\"48\">",
                escape(&notice.creator),
                escape(&notice.creator_id)
            )
        } else {
            format!(
                "<span class=\"avatar\" style=\"background-color: {};\">{}</span>",
                avatar_color(&notice.creator_id),
                escape(&initials(&notice.creator))
            )
        };
        if notice.kind == "Mention" {
            let first = notice
                .creator
                .split_whitespace()
                .next()
                .unwrap_or(&notice.creator);
            plain.push_str(&format!(
                "{first} mentioned you:\n{}\n{notice_url}\n\n",
                notice.detail
            ));
            html.push_str(&format!("<table class=\"notification\"><tr><td class=\"avatar__container\">{avatar}</td><td class=\"what\"><strong class=\"notificaton__mention\">{} mentioned you:</strong><p class=\"notification__details\">{}</p></td></tr></table>", escape(first), escape(&notice.detail)));
        } else {
            plain.push_str(&format!(
                "{}:\n{}\n{notice_url}\n\n",
                notice.creator, notice.detail
            ));
            let familiar = {
                let mut words = notice.creator.split_whitespace();
                let first = words.next().unwrap_or("");
                let suffix = words
                    .filter_map(|word| word.chars().next())
                    .map(|ch| format!("{ch}."))
                    .collect::<Vec<_>>()
                    .join("");
                if suffix.is_empty() {
                    first.to_owned()
                } else {
                    format!("{first}\u{a0}{suffix}")
                }
            };
            html.push_str(&format!("<table class=\"notification\"><tr><td class=\"avatar__container\">{avatar}</td><td class=\"what\"><p class=\"notification__author\">{}</p><p class=\"notification__details\">{}</p></td></tr></table>", escape(&familiar), escape(&notice.detail)));
        }
    }
    plain.push_str(&format!("\n--------------------------------------------------------------------------------\n\nRizzy emails you about new notifications every few hours.\n\nChange how often you get these:\n{settings}\n\nUnsubscribe from all email notifications:\n{unsubscribe_page}\n"));
    html.push_str(&format!("<p class=\"footer\">Rizzy emails you about new notifications every few hours. <br><a href=\"{}\">Change how often you get these</a> or <a href=\"{}\">unsubscribe from all email notifications</a>.</p></td></tr>", escape(settings), escape(unsubscribe_page)));
    (
        plain,
        include_str!("../assets/notification_mail_layout.html").replace("{{content}}", &html),
    )
}

async fn send_mail(
    address: &str,
    subject: &str,
    plain: &str,
    html: &str,
    unsubscribe: &str,
) -> Result<()> {
    let path = env::var("RIZZY_SENDMAIL").map_err(|_| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "Mail delivery is not configured",
        )
    })?;
    let from = env::var("RIZZY_MAIL_FROM").unwrap_or_else(|_| "no-reply@localhost".into());
    if !valid_email(address) || !valid_email(&from) || subject.contains(['\r', '\n']) {
        return Err(bad("Invalid mail address"));
    }
    let boundary = format!("rizzy-{}", id());
    let message = format!(
        "From: {from}\r\nTo: {address}\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"{boundary}\"\r\nList-Unsubscribe: <{unsubscribe}>\r\nList-Unsubscribe-Post: List-Unsubscribe=One-Click\r\n\r\n--{boundary}\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{}\r\n--{boundary}\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{}\r\n--{boundary}--\r\n",
        plain.replace('\n', "\r\n"),
        html.replace('\n', "\r\n")
    );
    let mut child = Command::new(path)
        .arg("-t")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"))?;
    let Some(mut stdin) = child.stdin.take() else {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"));
    };
    stdin
        .write_all(message.as_bytes())
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"))?;
    drop(stdin);
    let status = child
        .wait()
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"))?;
    if !status.success() {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"));
    }
    Ok(())
}

async fn deliver_one(app: &App, bundle: &DbRow) -> Result<()> {
    let user_id: String = bundle.get("user_id");
    let account_id: String = bundle.get("account_id");
    let Some(user) = sqlx::query("SELECT u.name,u.role,u.active,u.verified_at,i.email_address,a.name account_name,a.external_account_id,COALESCE(s.bundle_email_frequency,'every_few_hours') frequency,t.timezone_name FROM users u JOIN identities i ON i.id=u.identity_id JOIN accounts a ON a.id=u.account_id LEFT JOIN notification_settings s ON s.user_id=u.id LEFT JOIN user_timezones t ON t.user_id=u.id WHERE u.id=? AND u.account_id=? AND NOT EXISTS(SELECT 1 FROM account_cancellations c WHERE c.account_id=a.id)")
        .bind(&user_id).bind(&account_id).fetch_optional(&app.db).await.map_err(db_err)? else {
        return Ok(());
    };
    if user.get::<String, _>("frequency") == "never"
        || user.get::<String, _>("role") == "system"
        || user.get::<i64, _>("active") == 0
        || user.get::<Option<String>, _>("verified_at").is_none()
    {
        return Ok(());
    }
    let started: String = bundle.get("starts_at");
    let ended: String = bundle.get("ends_at");
    let notifications = mail_notices(app, &user_id, &started, &ended).await?;
    if notifications.is_empty() {
        return Ok(());
    }
    let token = unsubscribes::ensure_token(app, &user_id).await?;
    let host = env::var("RIZZY_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3007".into());
    let host = host.trim_end_matches('/');
    let slug: i64 = user.get("external_account_id");
    let unsubscribe = format!("{host}/{slug}/notifications/unsubscribe?access_token={token}");
    let settings = format!("{host}/{slug}/notifications/settings");
    let timezone: Option<String> = user.get("timezone_name");
    let unsubscribe_page =
        format!("{host}/{slug}/notifications/unsubscribe/new?access_token={token}");
    let email: String = user.get("email_address");
    let account_name: String = user.get("account_name");
    let account_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users u JOIN users other ON other.identity_id=u.identity_id WHERE u.id=?")
        .bind(&user_id).fetch_one(&app.db).await.map_err(db_err)?;
    let safe_account = account_name.replace(['\r', '\n'], " ");
    let subject = if account_count > 1 {
        format!("Rizzy ({safe_account}): New notifications")
    } else {
        "Rizzy: New notifications".into()
    };
    let (plain, html) = render_mail(
        &notifications,
        &display_time(&started, timezone.as_deref()),
        &account_name,
        account_count > 1,
        host,
        slug,
        &settings,
        &unsubscribe_page,
    );
    send_mail(&email, &subject, &plain, &html, &unsubscribe).await
}

pub(super) async fn deliver_due(app: &App) -> Result<usize> {
    if env::var("RIZZY_SENDMAIL").is_err() {
        return Ok(0);
    }
    let bundles = sqlx::query("SELECT id,user_id,account_id,starts_at,ends_at FROM notification_bundles WHERE status=0 AND ends_at<=? ORDER BY ends_at LIMIT 100")
        .bind(now()).fetch_all(&app.db).await.map_err(db_err)?;
    let mut delivered = 0;
    for bundle in bundles {
        let id: String = bundle.get("id");
        let claimed = sqlx::query(
            "UPDATE notification_bundles SET status=1,updated_at=? WHERE id=? AND status=0",
        )
        .bind(now())
        .bind(&id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
        if claimed.rows_affected() == 0 {
            continue;
        }
        match deliver_one(app, &bundle).await {
            Ok(()) => {
                sqlx::query("UPDATE notification_bundles SET status=2,updated_at=? WHERE id=?")
                    .bind(now())
                    .bind(&id)
                    .execute(&app.db)
                    .await
                    .map_err(db_err)?;
                delivered += 1;
            }
            Err(error) => {
                sqlx::query("UPDATE notification_bundles SET status=0,updated_at=? WHERE id=?")
                    .bind(now())
                    .bind(&id)
                    .execute(&app.db)
                    .await
                    .map_err(db_err)?;
                eprintln!("notification bundle delivery failed: {}", error.1);
            }
        }
    }
    Ok(delivered)
}

pub(super) async fn worker(app: Arc<App>) {
    let mut interval = tokio::time::interval(Duration::from_secs(60));
    loop {
        interval.tick().await;
        if let Err(error) = deliver_due(&app).await {
            eprintln!("notification bundle worker failed: {}", error.1);
        }
    }
}
