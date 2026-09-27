use super::*;

pub(super) async fn cleanup(app: &App) -> Result<usize> {
    let mut deleted = 0usize;
    loop {
        let cutoff = (Utc::now() - chrono::Duration::days(7))
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
        let statement = if app.mysql {
            "DELETE FROM webhook_deliveries WHERE created_at<? ORDER BY created_at,id LIMIT 500"
        } else {
            "DELETE FROM webhook_deliveries WHERE id IN (SELECT id FROM webhook_deliveries WHERE created_at<? ORDER BY created_at,id LIMIT 500)"
        };
        let count = sqlx::query(statement)
            .bind(cutoff)
            .execute(&app.db)
            .await
            .map_err(db_err)?
            .rows_affected() as usize;
        if count == 0 {
            break;
        }
        deleted += count;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    Ok(deleted)
}

async fn claim_quarter_hour(app: &App, slot: &str) -> Result<bool> {
    let insert = if app.mysql {
        "INSERT IGNORE INTO app_secrets(name,value) VALUES('webhook_cleanup_quarter_hour','0')"
    } else {
        "INSERT OR IGNORE INTO app_secrets(name,value) VALUES('webhook_cleanup_quarter_hour','0')"
    };
    sqlx::query(insert).execute(&app.db).await.map_err(db_err)?;
    let changed = sqlx::query(
        "UPDATE app_secrets SET value=? WHERE name='webhook_cleanup_quarter_hour' AND value<?",
    )
    .bind(slot)
    .bind(slot)
    .execute(&app.db)
    .await
    .map_err(db_err)?;
    Ok(changed.rows_affected() == 1)
}

pub(super) async fn worker(app: Arc<App>) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        let current = Utc::now();
        if current.minute() % 15 != 0 {
            continue;
        }
        let slot = current.format("%Y-%m-%dT%H:%M").to_string();
        match claim_quarter_hour(&app, &slot).await {
            Ok(true) => {
                if let Err(error) = cleanup(&app).await {
                    eprintln!("webhook delivery cleanup failed: {}", error.1);
                    if let Err(release) = sqlx::query("UPDATE app_secrets SET value='0' WHERE name='webhook_cleanup_quarter_hour' AND value=?")
                        .bind(&slot).execute(&app.db).await {
                        eprintln!("webhook cleanup claim release failed: {release}");
                    }
                }
            }
            Ok(false) => {}
            Err(error) => eprintln!("webhook cleanup claim failed: {}", error.1),
        }
    }
}
