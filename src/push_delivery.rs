use super::*;
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use futures_util::{StreamExt, stream};
use web_push::{
    ContentEncoding, SubscriptionInfo, Urgency, VapidSignatureBuilder, WebPushError,
    WebPushMessageBuilder,
};

fn private_key() -> Option<String> {
    env::var("RIZZY_VAPID_PRIVATE_KEY")
        .or_else(|_| env::var("VAPID_PRIVATE_KEY"))
        .ok()
        .filter(|s| !s.is_empty())
}

pub(super) fn public_key() -> Option<String> {
    private_key()
        .and_then(|key| VapidSignatureBuilder::from_base64_no_sub(&key).ok())
        .map(|builder| URL_SAFE_NO_PAD.encode(builder.get_public_key()))
}

pub(super) async fn queue(
    tx: &mut sqlx::Transaction<'_, Sqlite>,
    user_id: &str,
    card_id: &str,
) -> Result<()> {
    let timestamp = now();
    sqlx::query("INSERT OR IGNORE INTO push_deliveries(id,account_id,notification_id,source_id,status,attempts,next_attempt_at,created_at,updated_at) SELECT ?,n.account_id,n.id,n.source_id,0,0,?,?,? FROM notifications n WHERE n.user_id=? AND n.card_id=?")
        .bind(id()).bind(&timestamp).bind(&timestamp).bind(&timestamp).bind(user_id).bind(card_id)
        .execute(&mut **tx).await.map_err(db_err)?;
    Ok(())
}

fn excerpt(content: &str) -> String {
    let text = rich_text::plain_text(content);
    let text = text.chars().take(400).collect::<String>();
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() <= 200 {
        text
    } else {
        let mut short = text.chars().take(197).collect::<String>();
        if let Some(space) = short.rfind(char::is_whitespace) {
            short.truncate(space);
        }
        format!("{}...", short.trim_end())
    }
}

fn valid_subscription_keys(p256dh: &str, auth: &str) -> bool {
    let Ok(public) = URL_SAFE_NO_PAD.decode(p256dh) else {
        return false;
    };
    let Ok(secret) = URL_SAFE_NO_PAD.decode(auth) else {
        return false;
    };
    public.len() == 65 && public[0] == 4 && secret.len() == 16
}

async fn payload(app: &App, notification_id: &str) -> Result<Option<(String, String, Value)>> {
    let Some(row) = sqlx::query("SELECT n.id,n.account_id,n.user_id,n.source_type,n.source_id,n.card_id,c.title card_title,c.number,c.description,c.closed,a.external_account_id slug,u.active,creator.name creator_name,creator.role creator_role FROM notifications n JOIN cards c ON c.id=n.card_id JOIN accounts a ON a.id=n.account_id JOIN users u ON u.id=n.user_id JOIN users creator ON creator.id=n.creator_id WHERE n.id=? AND NOT EXISTS(SELECT 1 FROM account_cancellations x WHERE x.account_id=n.account_id)")
        .bind(notification_id).fetch_optional(&app.db).await.map_err(db_err)? else { return Ok(None); };
    if row.get::<i64, _>("active") == 0 || row.get::<String, _>("creator_role") == "system" {
        return Ok(None);
    }
    let user_id: String = row.get("user_id");
    let account_id: String = row.get("account_id");
    let source_id: String = row.get("source_id");
    let slug: i64 = row.get("slug");
    let number: i64 = row.get("number");
    let host = env::var("RIZZY_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3007".into());
    let host = host.trim_end_matches('/');
    let card_url = format!("{host}/{slug}/cards/{number}");
    let creator: String = row.get("creator_name");
    let card_title: String = row.get("card_title");
    let card_title = if card_title.trim().is_empty() {
        format!("Card {number}")
    } else {
        card_title
    };
    let (title, body, url) = if row.get::<String, _>("source_type") == "Mention" {
        let Some(mention) =
            sqlx::query("SELECT m.source_type,m.source_id FROM mentions m WHERE m.id=?")
                .bind(&source_id)
                .fetch_optional(&app.db)
                .await
                .map_err(db_err)?
        else {
            return Ok(None);
        };
        let content: String = if mention.get::<String, _>("source_type") == "Comment" {
            let Some(content) = sqlx::query_scalar("SELECT body FROM comments WHERE id=?")
                .bind(mention.get::<String, _>("source_id"))
                .fetch_optional(&app.db)
                .await
                .map_err(db_err)?
            else {
                return Ok(None);
            };
            content
        } else {
            row.get("description")
        };
        let first = creator.split_whitespace().next().unwrap_or(&creator);
        (
            format!("{first} mentioned you"),
            excerpt(&content),
            card_url,
        )
    } else {
        let Some(event) =
            sqlx::query("SELECT action,particulars,eventable_id FROM events WHERE id=?")
                .bind(&source_id)
                .fetch_optional(&app.db)
                .await
                .map_err(db_err)?
        else {
            return Ok(None);
        };
        let action: String = event.get("action");
        let particulars: Value =
            serde_json::from_str(&event.get::<String, _>("particulars")).unwrap_or_default();
        let details = &particulars["particulars"];
        let get = |key: &str| details[key].as_str().filter(|s| !s.trim().is_empty());
        let body = match action.as_str() {
            "comment_created" => {
                let comment: Option<String> =
                    sqlx::query_scalar("SELECT body FROM comments WHERE id=?")
                        .bind(event.get::<String, _>("eventable_id"))
                        .fetch_optional(&app.db)
                        .await
                        .map_err(db_err)?;
                let Some(comment) = comment else {
                    return Ok(None);
                };
                excerpt(&comment)
            }
            "card_assigned" => format!("Assigned to you by {creator}"),
            "card_published" => format!("Added by {creator}"),
            "card_closed" if row.get::<i64, _>("closed") != 0 => {
                format!("Moved to Done by {creator}")
            }
            "card_closed" => format!("Closed by {creator}"),
            "card_reopened" => format!("Reopened by {creator}"),
            "card_triaged" => get("column")
                .map(|s| format!("Moved to {s} by {creator}"))
                .unwrap_or_else(|| format!("Moved by {creator}")),
            "card_sent_back_to_triage" => format!("Moved back to Maybe? by {creator}"),
            "card_board_changed" | "card_collection_changed" => get("new_board")
                .or_else(|| get("new_collection"))
                .map(|s| format!("Moved to {s} by {creator}"))
                .unwrap_or_else(|| format!("Moved by {creator}")),
            "card_title_changed" => get("new_title")
                .map(|s| format!("Renamed to {s} by {creator}"))
                .unwrap_or_else(|| format!("Renamed by {creator}")),
            "card_postponed" => format!("Moved to Not Now by {creator}"),
            "card_auto_postponed" => "Moved to Not Now due to inactivity".into(),
            _ => format!("Updated by {creator}"),
        };
        let title = if action == "comment_created" {
            format!("RE: {card_title}")
        } else {
            card_title
        };
        let url = if action == "comment_created" {
            format!(
                "{card_url}#comment_{}",
                event.get::<String, _>("eventable_id")
            )
        } else {
            card_url
        };
        (title, body, url)
    };
    let badge: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notifications WHERE user_id=? AND read_at IS NULL",
    )
    .bind(&user_id)
    .fetch_one(&app.db)
    .await
    .map_err(db_err)?;
    Ok(Some((
        account_id,
        user_id,
        json!({"title":title,"options":{"body":body,"icon":"/apple-touch-icon.png","data":{"url":url,"path":url,"badge":badge}}}),
    )))
}

fn build_message(
    endpoint: &str,
    p256dh: &str,
    auth: &str,
    content: &[u8],
    key: &str,
) -> std::result::Result<web_push::WebPushMessage, WebPushError> {
    let subscription =
        SubscriptionInfo::new(endpoint.to_owned(), p256dh.to_owned(), auth.to_owned());
    let mut signature = VapidSignatureBuilder::from_base64(key, &subscription)?;
    let subject = env::var("RIZZY_VAPID_SUBJECT").unwrap_or_else(|_| {
        let public = env::var("RIZZY_PUBLIC_URL").unwrap_or_default();
        if public.starts_with("https://") {
            public
        } else {
            "mailto:admin@localhost".into()
        }
    });
    signature.add_claim("sub", subject);
    let mut builder = WebPushMessageBuilder::new(&subscription);
    builder.set_payload(ContentEncoding::Aes128Gcm, content);
    builder.set_vapid_signature(signature.build()?);
    builder.set_urgency(Urgency::High);
    builder.build()
}

async fn send(
    endpoint: &str,
    p256dh: &str,
    auth: &str,
    content: &[u8],
    key: &str,
) -> std::result::Result<bool, String> {
    let (host, pinned) = push_subscriptions::delivery_address(endpoint)
        .await
        .map_err(|e| e.1)?;
    let message = match build_message(endpoint, p256dh, auth, content, key) {
        Ok(message) => message,
        Err(WebPushError::InvalidCryptoKeys | WebPushError::MissingCryptoKeys) => return Ok(false),
        Err(WebPushError::PayloadTooLarge) => {
            eprintln!("push payload exceeded the Web Push size limit");
            return Ok(true);
        }
        Err(error) => return Err(error.to_string()),
    };
    let client = reqwest::Client::builder()
        .resolve(&host, pinned)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let mut request = client
        .post(endpoint)
        .header("TTL", message.ttl.to_string())
        .header("Urgency", "high");
    let Some(encrypted) = message.payload else {
        return Err("Missing encrypted push payload".into());
    };
    request = request
        .header("Content-Encoding", encrypted.content_encoding.to_str())
        .header("Content-Type", "application/octet-stream");
    for (name, value) in encrypted.crypto_headers {
        request = request.header(name, value);
    }
    let response = request
        .body(encrypted.content)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if matches!(response.status().as_u16(), 404 | 410) {
        return Ok(false);
    }
    if response.status().is_success() {
        Ok(true)
    } else {
        Err(format!("Push service returned {}", response.status()))
    }
}

async fn deliver_one(
    app: &App,
    delivery_id: &str,
    notification_id: &str,
    source_id: &str,
    key: &str,
) -> Result<bool> {
    let Some((account_id, user_id, value)) = payload(app, notification_id).await? else {
        return Ok(true);
    };
    let current_source: Option<String> =
        sqlx::query_scalar("SELECT source_id FROM notifications WHERE id=?")
            .bind(notification_id)
            .fetch_optional(&app.db)
            .await
            .map_err(db_err)?;
    if current_source.as_deref() != Some(source_id) {
        return Ok(true);
    }
    let subscriptions = sqlx::query("SELECT id,endpoint,p256dh_key,auth_key FROM push_subscriptions WHERE account_id=? AND user_id=?")
        .bind(&account_id).bind(&user_id).fetch_all(&app.db).await.map_err(db_err)?;
    let content = serde_json::to_vec(&value)
        .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Invalid push payload"))?;
    let mut retry = false;
    for subscription in subscriptions {
        let id: String = subscription.get("id");
        let sent: i64 = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM push_delivery_receipts WHERE delivery_id=? AND subscription_id=?)")
            .bind(delivery_id).bind(&id).fetch_one(&app.db).await.map_err(db_err)?;
        if sent != 0 {
            continue;
        }
        let endpoint: String = subscription.get("endpoint");
        let p256dh: Option<String> = subscription.get("p256dh_key");
        let auth: Option<String> = subscription.get("auth_key");
        let (Some(p256dh), Some(auth)) = (p256dh, auth) else {
            continue;
        };
        if !valid_subscription_keys(&p256dh, &auth) {
            sqlx::query("DELETE FROM push_subscriptions WHERE id=? AND user_id=?")
                .bind(&id)
                .bind(&user_id)
                .execute(&app.db)
                .await
                .map_err(db_err)?;
            continue;
        }
        sqlx::query("UPDATE push_deliveries SET updated_at=? WHERE id=? AND status=1")
            .bind(now())
            .bind(delivery_id)
            .execute(&app.db)
            .await
            .map_err(db_err)?;
        match send(&endpoint, &p256dh, &auth, &content, key).await {
            Ok(true) => {
                sqlx::query("INSERT OR IGNORE INTO push_delivery_receipts(delivery_id,subscription_id) VALUES(?,?)")
                    .bind(delivery_id).bind(&id).execute(&app.db).await.map_err(db_err)?;
            }
            Ok(false) => {
                sqlx::query("DELETE FROM push_subscriptions WHERE id=? AND user_id=?")
                    .bind(id)
                    .bind(&user_id)
                    .execute(&app.db)
                    .await
                    .map_err(db_err)?;
            }
            Err(error) => {
                eprintln!("push delivery to subscription {id} failed: {error}");
                retry = true;
            }
        }
    }
    Ok(!retry)
}

async fn process_job(app: &App, row: SqliteRow, key: &str) -> Result<usize> {
    let id: String = row.get("id");
    let claimed = sqlx::query("UPDATE push_deliveries SET status=1,attempts=attempts+1,updated_at=? WHERE id=? AND status=0")
        .bind(now()).bind(&id).execute(&app.db).await.map_err(db_err)?;
    if claimed.rows_affected() == 0 {
        return Ok(0);
    }
    let success = match deliver_one(
        app,
        &id,
        &row.get::<String, _>("notification_id"),
        &row.get::<String, _>("source_id"),
        key,
    )
    .await
    {
        Ok(done) => done,
        Err(error) => {
            eprintln!("push delivery {id} failed: {}", error.1);
            false
        }
    };
    if success {
        sqlx::query("UPDATE push_deliveries SET status=2,updated_at=? WHERE id=?")
            .bind(now())
            .bind(&id)
            .execute(&app.db)
            .await
            .map_err(db_err)?;
        Ok(1)
    } else {
        let attempts: i64 = row.get("attempts");
        let delay = 60 * (1_i64 << attempts.min(6));
        let next = (Utc::now() + chrono::Duration::seconds(delay))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        sqlx::query(
            "UPDATE push_deliveries SET status=0,next_attempt_at=?,updated_at=? WHERE id=?",
        )
        .bind(next)
        .bind(now())
        .bind(&id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
        Ok(0)
    }
}

pub(super) async fn deliver_due(app: &App) -> Result<usize> {
    let Some(key) = private_key() else {
        sqlx::query("UPDATE push_deliveries SET status=2,updated_at=? WHERE id IN (SELECT id FROM push_deliveries WHERE status=0 AND next_attempt_at<=? LIMIT 1000)")
            .bind(now()).bind(now()).execute(&app.db).await.map_err(db_err)?;
        return Ok(0);
    };
    VapidSignatureBuilder::from_base64_no_sub(&key)
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Invalid VAPID private key"))?;
    let stale = (Utc::now() - chrono::Duration::minutes(5))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    sqlx::query("UPDATE push_deliveries SET status=0 WHERE status=1 AND updated_at<?")
        .bind(stale)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    let rows = sqlx::query("SELECT id,notification_id,source_id,attempts FROM push_deliveries WHERE status=0 AND next_attempt_at<=? ORDER BY next_attempt_at LIMIT 100")
        .bind(now()).fetch_all(&app.db).await.map_err(db_err)?;
    let mut jobs =
        stream::iter(rows.into_iter().map(|row| process_job(app, row, &key))).buffer_unordered(50);
    let mut delivered = 0;
    while let Some(result) = jobs.next().await {
        delivered += result?;
    }
    Ok(delivered)
}

pub(super) async fn worker(app: Arc<App>) {
    let mut interval = tokio::time::interval(Duration::from_secs(5));
    loop {
        interval.tick().await;
        if let Err(error) = deliver_due(&app).await {
            eprintln!("push delivery worker failed: {}", error.1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{App, build_message, payload, process_job};

    #[test]
    fn encrypts_signed_browser_payload_with_high_urgency() {
        let endpoint = "https://fcm.googleapis.com/fcm/send/test";
        let public = "BGa4N1PI79lboMR_YrwCiCsgp35DRvedt7opHcf0yM3iOBTSoQYqQLwWxAfRKE6tsDnReWmhsImkhDF_DBdkNSU";
        let auth = "EvcWjEgzr4rbvhfi3yds0A";
        let private = "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY";
        let content = br#"{"title":"A card","options":{"body":"Added by Ada"}}"#;
        let message = build_message(endpoint, public, auth, content, private).unwrap();
        assert_eq!(message.urgency, Some(web_push::Urgency::High));
        let encrypted = message.payload.unwrap();
        assert_eq!(encrypted.content_encoding.to_str(), "aes128gcm");
        assert!(encrypted.content.len() > content.len());
        assert_ne!(&encrypted.content[..content.len()], content);
        assert!(
            encrypted
                .crypto_headers
                .iter()
                .any(|(name, _)| *name == "Authorization")
        );
    }

    #[tokio::test]
    async fn event_and_mention_payloads_follow_fizzy_action_text() {
        let db = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::raw_sql(include_str!("../schema.sql"))
            .execute(&db)
            .await
            .unwrap();
        sqlx::raw_sql("INSERT INTO accounts(id,external_account_id,name,created_at) VALUES('a',7,'Team','2026-01-01');
            INSERT INTO identities(id,email_address,created_at) VALUES('i1','one@example.test','2026-01-01'),('i2','two@example.test','2026-01-01');
            INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('author','a','i1','Ada Lovelace','member','2026-01-01'),('recipient','a','i2','Bob Example','member','2026-01-01');
            INSERT INTO boards(id,account_id,creator_id,name,created_at,updated_at) VALUES('b','a','author','Board','2026-01-01','2026-01-01');
            INSERT INTO cards(id,account_id,board_id,creator_id,number,title,description,created_at,updated_at,last_active_at) VALUES('c','a','b','author',3,'A card','<p>Hello Bob</p>','2026-01-01','2026-01-01','2026-01-01');
            INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,created_at) VALUES('e','a','b','c','author','Card','c','card_published','2026-01-01');
            INSERT INTO notifications(id,account_id,user_id,creator_id,card_id,source_id,created_at,updated_at) VALUES('n','a','recipient','author','c','e','2026-01-01','2026-01-01');")
            .execute(&db).await.unwrap();
        let app = App {
            db,
            qr_secret: "test-qr-secret".into(),
            write_lock: tokio::sync::Mutex::new(()),
            auth_lock: tokio::sync::Mutex::new(()),
            notification_html_cache: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            notification_html_build_locks: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            events_html_cache: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            events_html_build_locks: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            board_etag_connection: None,
            board_etag_cache: tokio::sync::Mutex::new(std::collections::HashMap::new()),
            export_notify: tokio::sync::Notify::new(),
            import_notify: tokio::sync::Notify::new(),
        };
        let (_, _, value) = payload(&app, "n").await.unwrap().unwrap();
        assert_eq!(value["title"], "A card");
        assert_eq!(value["options"]["body"], "Added by Ada Lovelace");
        assert_eq!(value["options"]["data"]["badge"], 1);
        sqlx::query("UPDATE events SET action='card_triaged',particulars=? WHERE id='e'")
            .bind(r#"{"particulars":{"column":"In Progress"}}"#)
            .execute(&app.db)
            .await
            .unwrap();
        let (_, _, value) = payload(&app, "n").await.unwrap().unwrap();
        assert_eq!(
            value["options"]["body"],
            "Moved to In Progress by Ada Lovelace"
        );
        for (action, particulars, expected) in [
            ("card_assigned", "{}", "Assigned to you by Ada Lovelace"),
            (
                "card_board_changed",
                r#"{"particulars":{"new_board":"Another board"}}"#,
                "Moved to Another board by Ada Lovelace",
            ),
            (
                "card_title_changed",
                r#"{"particulars":{"new_title":"New title"}}"#,
                "Renamed to New title by Ada Lovelace",
            ),
            ("card_postponed", "{}", "Moved to Not Now by Ada Lovelace"),
            (
                "card_auto_postponed",
                "{}",
                "Moved to Not Now due to inactivity",
            ),
            ("card_unassigned", "{}", "Updated by Ada Lovelace"),
        ] {
            sqlx::query("UPDATE events SET action=?,particulars=? WHERE id='e'")
                .bind(action)
                .bind(particulars)
                .execute(&app.db)
                .await
                .unwrap();
            let (_, _, value) = payload(&app, "n").await.unwrap().unwrap();
            assert_eq!(value["options"]["body"], expected, "{action}");
        }
        sqlx::raw_sql("INSERT INTO comments(id,card_id,creator_id,body,created_at) VALUES('comment1','c','author','<p>A short comment</p>','2026-01-01');
            UPDATE events SET action='comment_created',eventable_type='Comment',eventable_id='comment1' WHERE id='e';")
            .execute(&app.db).await.unwrap();
        let (_, _, value) = payload(&app, "n").await.unwrap().unwrap();
        assert_eq!(value["title"], "RE: A card");
        assert_eq!(value["options"]["body"], "A short comment");
        assert!(
            value["options"]["data"]["url"]
                .as_str()
                .unwrap()
                .ends_with("/7/cards/3#comment_comment1")
        );
        sqlx::raw_sql("INSERT INTO mentions(id,account_id,mentionee_id,mentioner_id,source_type,source_id,created_at,updated_at) VALUES('m','a','recipient','author','Card','c','2026-01-01','2026-01-01');
            UPDATE notifications SET source_type='Mention',source_id='m' WHERE id='n';")
            .execute(&app.db).await.unwrap();
        let (_, _, value) = payload(&app, "n").await.unwrap().unwrap();
        assert_eq!(value["title"], "Ada mentioned you");
        assert_eq!(value["options"]["body"], "Hello Bob");
        assert_eq!(
            value["options"]["data"]["url"],
            "http://localhost:3007/7/cards/3"
        );
        sqlx::raw_sql("INSERT INTO push_subscriptions(id,account_id,user_id,endpoint,p256dh_key,auth_key,created_at,updated_at) VALUES('s','a','recipient','https://fcm.googleapis.com/fcm/send/test','bad','keys','2026-01-01','2026-01-01');
            INSERT INTO push_deliveries(id,account_id,notification_id,source_id,next_attempt_at,created_at,updated_at) VALUES('d','a','n','m','2026-01-01','2026-01-01','2026-01-01');")
            .execute(&app.db).await.unwrap();
        let row = sqlx::query(
            "SELECT id,notification_id,source_id,attempts FROM push_deliveries WHERE id='d'",
        )
        .fetch_one(&app.db)
        .await
        .unwrap();
        assert_eq!(
            process_job(&app, row, "IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY")
                .await
                .unwrap(),
            1
        );
        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM push_subscriptions WHERE id='s'")
                .fetch_one(&app.db)
                .await
                .unwrap();
        assert_eq!(remaining, 0);
    }
}
