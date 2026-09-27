use super::*;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CleanupJob {
    MagicLinks,
    Transfers,
    Exports,
    Imports,
}

impl CleanupJob {
    fn claim_name(self) -> &'static str {
        match self {
            Self::MagicLinks => "magic_link_cleanup_slot",
            Self::Transfers => "transfer_cleanup_slot",
            Self::Exports => "export_cleanup_slot",
            Self::Imports => "import_cleanup_slot",
        }
    }
}

fn due_jobs<Tz: chrono::TimeZone>(current: chrono::DateTime<Tz>) -> Vec<CleanupJob> {
    match current.minute() {
        0 if current.hour() % 4 == 0 => {
            vec![CleanupJob::MagicLinks, CleanupJob::Transfers]
        }
        20 => vec![CleanupJob::Exports],
        25 => vec![CleanupJob::Imports],
        _ => Vec::new(),
    }
}

pub(super) fn after_start_minute<Tz: chrono::TimeZone>(
    started_utc_minute: i64,
    current: &chrono::DateTime<Tz>,
) -> bool {
    current.timestamp().div_euclid(60) > started_utc_minute
}

async fn claim_slot(
    db: &DbPool,
    mysql: bool,
    job: CleanupJob,
    slot: &str,
) -> std::result::Result<bool, sqlx::Error> {
    let insert = if mysql {
        "INSERT IGNORE INTO app_secrets(name,value) VALUES(?,'0')"
    } else {
        "INSERT OR IGNORE INTO app_secrets(name,value) VALUES(?,'0')"
    };
    sqlx::query(insert)
        .bind(job.claim_name())
        .execute(db)
        .await?;
    let changed = sqlx::query("UPDATE app_secrets SET value=? WHERE name=? AND value<?")
        .bind(slot)
        .bind(job.claim_name())
        .bind(slot)
        .execute(db)
        .await?;
    Ok(changed.rows_affected() == 1)
}

async fn cleanup(app: &App, job: CleanupJob) -> std::result::Result<(), sqlx::Error> {
    match job {
        CleanupJob::MagicLinks => {
            sqlx::query("DELETE FROM magic_links WHERE expires_at<=?")
                .bind(now())
                .execute(&app.db)
                .await?;
        }
        CleanupJob::Transfers => {
            let cutoff = (Utc::now() - chrono::Duration::hours(4))
                .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
            sqlx::query("DELETE FROM identity_transfers WHERE created_at<?")
                .bind(cutoff)
                .execute(&app.db)
                .await?;
        }
        CleanupJob::Exports => {
            exports::cleanup(app).await?;
        }
        CleanupJob::Imports => {
            imports::cleanup(app).await?;
        }
    }
    Ok(())
}

pub(super) async fn run_now(app: &App, name: &str) -> std::result::Result<(), sqlx::Error> {
    let job = match name {
        "cleanup-magic-links" => CleanupJob::MagicLinks,
        "cleanup-transfers" => CleanupJob::Transfers,
        "cleanup-exports" => CleanupJob::Exports,
        "cleanup-imports" => CleanupJob::Imports,
        _ => return Err(sqlx::Error::Protocol("unknown cleanup command".into())),
    };
    cleanup(app, job).await
}

pub(super) async fn worker(app: Arc<App>) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    let started_utc_minute = Utc::now().timestamp().div_euclid(60);
    loop {
        interval.tick().await;
        let current = Local::now();
        if !after_start_minute(started_utc_minute, &current) {
            continue;
        }
        let slot = current
            .with_timezone(&Utc)
            .format("%Y-%m-%dT%H")
            .to_string();
        for job in due_jobs(current) {
            match claim_slot(&app.db, app.mysql, job, &slot).await {
                Ok(true) => {
                    if let Err(error) = cleanup(&app, job).await {
                        eprintln!("{job:?} cleanup failed: {error}");
                        if let Err(release) =
                            sqlx::query("UPDATE app_secrets SET value='0' WHERE name=? AND value=?")
                                .bind(job.claim_name())
                                .bind(&slot)
                                .execute(&app.db)
                                .await
                        {
                            eprintln!("{job:?} cleanup claim release failed: {release}");
                        }
                    }
                }
                Ok(false) => {}
                Err(error) => eprintln!("{job:?} cleanup claim failed: {error}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{CleanupJob, after_start_minute, claim_slot, due_jobs};
    use chrono::{FixedOffset, TimeZone, Utc};
    use sqlx::any::AnyPoolOptions;

    #[test]
    fn source_cleanup_clock_slots() {
        for hour in 0..24 {
            for minute in [0, 1, 19, 20, 21, 24, 25, 26] {
                let current = Utc.with_ymd_and_hms(2026, 9, 27, hour, minute, 0).unwrap();
                let jobs = due_jobs(current);
                assert_eq!(
                    jobs.contains(&CleanupJob::MagicLinks),
                    minute == 0 && hour % 4 == 0
                );
                assert_eq!(
                    jobs.contains(&CleanupJob::Transfers),
                    minute == 0 && hour % 4 == 0
                );
                assert_eq!(jobs.contains(&CleanupJob::Exports), minute == 20);
                assert_eq!(jobs.contains(&CleanupJob::Imports), minute == 25);
            }
        }
        let chicago = FixedOffset::west_opt(5 * 3600).unwrap();
        let local = chicago.with_ymd_and_hms(2026, 9, 27, 8, 0, 0).unwrap();
        assert_eq!(
            due_jobs(local),
            vec![CleanupJob::MagicLinks, CleanupJob::Transfers]
        );
    }

    #[test]
    fn startup_minute_is_not_a_due_clock_occurrence() {
        let started = Utc.with_ymd_and_hms(2026, 9, 27, 8, 20, 5).unwrap();
        let minute = started.timestamp().div_euclid(60);
        assert!(!after_start_minute(minute, &started));
        assert!(!after_start_minute(
            minute,
            &(started + chrono::Duration::seconds(40))
        ));
        assert!(after_start_minute(
            minute,
            &(started + chrono::Duration::seconds(55))
        ));
    }

    #[tokio::test]
    async fn one_process_claims_each_shared_cleanup_slot() {
        sqlx::any::install_default_drivers();
        let path = std::env::temp_dir().join(format!("rizzy-cleanup-{}.sqlite3", super::id()));
        std::fs::File::create(&path).unwrap();
        let url = format!("sqlite://{}", path.display());
        let db = AnyPoolOptions::new()
            .max_connections(4)
            .connect(&url)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE app_secrets(name TEXT PRIMARY KEY,value TEXT NOT NULL)")
            .execute(&db)
            .await
            .unwrap();
        let slot = "2026-09-27T08";
        let (a, b) = tokio::join!(
            claim_slot(&db, false, CleanupJob::MagicLinks, slot),
            claim_slot(&db, false, CleanupJob::MagicLinks, slot)
        );
        assert_eq!(
            [a.unwrap(), b.unwrap()]
                .iter()
                .filter(|claimed| **claimed)
                .count(),
            1
        );
        assert!(
            !claim_slot(&db, false, CleanupJob::MagicLinks, slot)
                .await
                .unwrap()
        );
        assert!(
            claim_slot(&db, false, CleanupJob::Transfers, slot)
                .await
                .unwrap()
        );
        assert!(
            claim_slot(&db, false, CleanupJob::MagicLinks, "2026-09-27T12")
                .await
                .unwrap()
        );
        db.close().await;
        std::fs::remove_file(path).unwrap();
    }
}
