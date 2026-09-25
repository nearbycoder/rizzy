use super::*;

pub(super) async fn create(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    if p.role != "owner" {
        return Err(err(StatusCode::FORBIDDEN, "Account owner required"));
    }
    // Fizzy allows cancellation only while accepting new account signups.
    if env::var("RIZZY_MULTI_TENANT").as_deref() == Ok("1") {
        let stamp = now();
        let inserted = sqlx::query("INSERT OR IGNORE INTO account_cancellations(id,account_id,initiated_by_id,created_at,updated_at) VALUES(?,?,?,?,?)")
            .bind(id())
            .bind(&p.account_id)
            .bind(&p.user_id)
            .bind(&stamp)
            .bind(&stamp)
            .execute(&app.db)
            .await
            .map_err(db_err)?;
        if inserted.rows_affected() == 1 {
            let recipient = sqlx::query("SELECT a.name,i.email_address FROM accounts a JOIN users u ON u.account_id=a.id JOIN identities i ON i.id=u.identity_id WHERE u.id=?")
                .bind(&p.user_id).fetch_one(&app.db).await.map_err(db_err)?;
            let name: String = recipient.get("name");
            let email: String = recipient.get("email_address");
            tokio::spawn(async move {
                if let Err(error) = send_notice(&email, &name).await {
                    eprintln!("account cancellation email failed: {error:?}");
                }
            });
        }
    }
    Ok((StatusCode::FOUND, [(header::LOCATION, "/session/menu")]).into_response())
}

async fn send_notice(email: &str, name: &str) -> Result<()> {
    let Ok(sendmail) = env::var("RIZZY_SENDMAIL") else {
        return Ok(());
    };
    let from = env::var("RIZZY_MAIL_FROM").unwrap_or_else(|_| "no-reply@localhost".into());
    if !valid_email(email) || from.contains(['\r', '\n']) {
        return Err(bad("Invalid mail address"));
    }
    let message = format!(
        "From: {from}\nTo: {email}\nSubject: Your Rizzy account was cancelled\nContent-Type: text/plain; charset=UTF-8\n\nYour Rizzy account \"{name}\" was cancelled.\n\nNo one can access the account anymore. Everything in the account will be deleted in 30 days.\n"
    );
    let mut child = Command::new(sendmail)
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
    if !child
        .wait()
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"))?
        .success()
    {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"));
    }
    Ok(())
}

pub(super) async fn menu(State(app): Shared, headers: HeaderMap) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let (identity_id, _) = identity_auth(&app, &headers).await?;
    let accounts = sqlx::query("SELECT a.external_account_id,a.name FROM accounts a JOIN users u ON u.account_id=a.id WHERE u.identity_id=? AND u.active=1 AND NOT EXISTS (SELECT 1 FROM account_cancellations c WHERE c.account_id=a.id) ORDER BY a.created_at")
        .bind(identity_id)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
    if accounts.len() == 1 {
        let slug: i64 = accounts[0].get("external_account_id");
        return Ok((StatusCode::FOUND, [(header::LOCATION, format!("/{slug}/"))]).into_response());
    }
    let mut body = String::from(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>Choose an account | Rizzy</title></head><body><main>",
    );
    if accounts.is_empty() {
        body.push_str("<h1>Hmm...</h1><p>You don’t have any Rizzy accounts.</p>");
    } else {
        body.push_str("<h1>Your Rizzy accounts</h1><menu>");
        for account in accounts {
            let slug: i64 = account.get("external_account_id");
            let name: String = account.get("name");
            body.push_str(&format!(
                "<li><a href=\"/{slug}/landing\">{}</a></li>",
                escape(&name)
            ));
        }
        body.push_str("</menu>");
    }
    body.push_str(
        "<a href=\"/signup/new\">Sign up for a new Rizzy account</a></main></body></html>",
    );
    Ok(Html(body).into_response())
}

// Runs on startup and hourly. All database records are removed in one transaction;
// SQLite checks deferred references at commit so user/card/board deletion order is safe.
pub(super) async fn incinerate_due(app: &App) -> std::result::Result<(), sqlx::Error> {
    let cutoff = (Utc::now() - chrono::Duration::days(30))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let due: Vec<String> = sqlx::query_scalar(
        "SELECT account_id FROM account_cancellations WHERE created_at<? ORDER BY created_at",
    )
    .bind(cutoff)
    .fetch_all(&app.db)
    .await?;
    for account_id in due {
        incinerate_one(app, &account_id).await?;
    }
    Ok(())
}

async fn incinerate_one(app: &App, account_id: &str) -> std::result::Result<(), sqlx::Error> {
    let mut tx = app.db.begin().await?;
    sqlx::query("PRAGMA defer_foreign_keys=ON")
        .execute(&mut *tx)
        .await?;
    let still_due: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM account_cancellations WHERE account_id=? AND created_at<?",
    )
    .bind(account_id)
    .bind(
        (Utc::now() - chrono::Duration::days(30))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    )
    .fetch_one(&mut *tx)
    .await?;
    if still_due == 0 {
        return Ok(());
    }
    let keys: Vec<String> =
        sqlx::query_scalar("SELECT storage_key FROM storage_blobs WHERE account_id=?")
            .bind(account_id)
            .fetch_all(&mut *tx)
            .await?;
    let export_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM exports WHERE account_id=?")
        .bind(account_id)
        .fetch_all(&mut *tx)
        .await?;
    let import_ids: Vec<String> = sqlx::query_scalar("SELECT id FROM imports WHERE account_id=?")
        .bind(account_id)
        .fetch_all(&mut *tx)
        .await?;
    let system_identity_ids: Vec<String> = sqlx::query_scalar("SELECT identity_id FROM users WHERE account_id=? AND role='system' AND identity_id IS NOT NULL")
        .bind(account_id).fetch_all(&mut *tx).await?;

    // Only account-scoped tables may be cleared. sqlite_master and table_info
    // supply trusted schema names, including tables added in future migrations.
    let tables: Vec<String> = sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '%_fts%' ORDER BY name")
        .fetch_all(&mut *tx).await?;
    for table in tables {
        if table == "accounts" {
            continue;
        }
        let quoted = format!("\"{}\"", table.replace('"', "\"\""));
        let columns = sqlx::query(&format!("PRAGMA table_info({quoted})"))
            .fetch_all(&mut *tx)
            .await?;
        if columns
            .iter()
            .any(|column| column.get::<String, _>("name") == "account_id")
        {
            sqlx::query(&format!("DELETE FROM {quoted} WHERE account_id=?"))
                .bind(account_id)
                .execute(&mut *tx)
                .await?;
        }
    }
    sqlx::query("DELETE FROM accounts WHERE id=?")
        .bind(account_id)
        .execute(&mut *tx)
        .await?;
    for identity_id in system_identity_ids {
        sqlx::query("DELETE FROM identities WHERE id=? AND NOT EXISTS (SELECT 1 FROM users WHERE identity_id=?)")
            .bind(&identity_id).bind(&identity_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;

    storage::remove_files(keys).await;
    for id in export_ids {
        let _ = tokio::fs::remove_file(exports::export_path(&id)).await;
    }
    for id in import_ids {
        let _ = tokio::fs::remove_file(imports::import_path(&id)).await;
    }
    Ok(())
}
