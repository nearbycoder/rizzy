use super::*;
use chrono::TimeZone;
use chrono_tz::Tz;

pub(super) async fn queue(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
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
        .and_then(|name| match name {
            "Madrid" => Some(chrono_tz::Europe::Madrid),
            "Eastern Time (US & Canada)" => Some(chrono_tz::America::New_York),
            "Central Time (US & Canada)" => Some(chrono_tz::America::Chicago),
            "Pacific Time (US & Canada)" => Some(chrono_tz::America::Los_Angeles),
            _ => name.parse::<Tz>().ok(),
        })
        .unwrap_or(chrono_tz::UTC);
    let local = zone.from_utc_datetime(&parsed.naive_utc());
    local.format("%-I%P on %A, %B %-d").to_string()
}

#[cfg(test)]
mod tests {
    use super::display_time;

    #[test]
    fn formats_bundle_start_in_rails_madrid_timezone() {
        assert_eq!(
            display_time("2025-01-15T14:30:00.000Z", Some("Madrid")),
            "3pm on Wednesday, January 15"
        );
    }
}

async fn send_mail(address: &str, subject: &str, body: &str, unsubscribe: &str) -> Result<()> {
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
    let message = format!(
        "From: {from}\nTo: {address}\nSubject: {subject}\nContent-Type: text/plain; charset=UTF-8\nList-Unsubscribe: <{unsubscribe}>\nList-Unsubscribe-Post: List-Unsubscribe=One-Click\n\n{body}"
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

async fn deliver_one(app: &App, bundle: &SqliteRow) -> Result<()> {
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
    let notifications = sqlx::query("SELECT n.id,n.source_type,n.updated_at,c.title,c.number,c.id card_id,b.name board_name,creator.name creator_name FROM notifications n JOIN cards c ON c.id=n.card_id JOIN boards b ON b.id=c.board_id JOIN users creator ON creator.id=n.creator_id WHERE n.user_id=? AND n.read_at IS NULL AND n.updated_at>=? AND n.updated_at<=? ORDER BY lower(b.name),c.number,n.updated_at")
        .bind(&user_id).bind(bundle.get::<String,_>("starts_at"))
        .bind(bundle.get::<String,_>("ends_at"))
        .fetch_all(&app.db).await.map_err(db_err)?;
    if notifications.is_empty() {
        return Ok(());
    }
    let token = unsubscribes::ensure_token(app, &user_id).await?;
    let host = env::var("RIZZY_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3007".into());
    let host = host.trim_end_matches('/');
    let slug: i64 = user.get("external_account_id");
    let unsubscribe = format!("{host}/{slug}/notifications/unsubscribe?access_token={token}");
    let settings = format!("{host}/{slug}/notifications/settings");
    let started: String = bundle.get("starts_at");
    let timezone: Option<String> = user.get("timezone_name");
    let mut body = format!(
        "Notifications since {}\nYou have {} new notification{}.\n\n",
        display_time(&started, timezone.as_deref()),
        notifications.len(),
        if notifications.len() == 1 { "" } else { "s" }
    );
    let mut last_board = String::new();
    let mut last_card = String::new();
    for notification in notifications {
        let board: String = notification.get("board_name");
        if board != last_board {
            body.push_str(&format!(
                "{board}\n{}\n\n",
                "-".repeat(board.chars().count())
            ));
            last_board = board;
        }
        let card_id: String = notification.get("card_id");
        let number: i64 = notification.get("number");
        if card_id != last_card {
            body.push_str(&format!(
                "#{number} {}\n",
                notification.get::<String, _>("title")
            ));
            last_card = card_id.clone();
        }
        body.push_str(&format!(
            "{}: {}\n{host}/{slug}/cards/{number}\n\n",
            notification.get::<String, _>("creator_name"),
            if notification.get::<String, _>("source_type") == "Mention" {
                "Mentioned you"
            } else {
                "New activity"
            }
        ));
    }
    body.push_str(&format!(
        "Change how often you get these: {settings}\nUnsubscribe from all email notifications: {host}/{slug}/notifications/unsubscribe/new?access_token={token}\n"
    ));
    let email: String = user.get("email_address");
    let account_name: String = user.get("account_name");
    let account_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users u JOIN users other ON other.identity_id=u.identity_id WHERE u.id=? AND other.active=1")
        .bind(&user_id).fetch_one(&app.db).await.map_err(db_err)?;
    let safe_account = account_name.replace(['\r', '\n'], " ");
    let subject = if account_count > 1 {
        format!("Rizzy ({safe_account}): New notifications")
    } else {
        "Rizzy: New notifications".into()
    };
    send_mail(&email, &subject, &body, &unsubscribe).await
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
