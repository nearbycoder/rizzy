use super::*;

async fn delete_filter(tx: &mut sqlx::Transaction<'_, Db>, filter_id: &str) -> Result<()> {
    // Remove links explicitly so filter deletion also works during schema repair.
    for table in [
        "filters_tags",
        "boards_filters",
        "assignees_filters",
        "creators_filters",
        "closers_filters",
    ] {
        sqlx::query(&format!("DELETE FROM {table} WHERE filter_id=?"))
            .bind(filter_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
    }
    sqlx::query("DELETE FROM filters WHERE id=?")
        .bind(filter_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    Ok(())
}

// Tag.destroy! removes the tag from saved filters before deleting it. Keep the
// same observable filter behavior even though Rizzy stores filter fields as JSON.
pub(super) async fn delete_unused_tags(app: &App) -> Result<usize> {
    let mut deleted = 0;
    let mut after = String::new();
    loop {
        let ids: Vec<String> = sqlx::query_scalar(
            "SELECT t.id FROM tags t WHERE t.id>? AND NOT EXISTS \
             (SELECT 1 FROM taggings g WHERE g.tag_id=t.id) ORDER BY t.id LIMIT 100",
        )
        .bind(&after)
        .fetch_all(&app.db)
        .await
        .map_err(db_err)?;
        if ids.is_empty() {
            break;
        }
        for tag_id in &ids {
            let mut tx = begin_write(app).await.map_err(db_err)?;
            let lock = if app.mysql {
                "SELECT id FROM tags WHERE id=? FOR UPDATE"
            } else {
                "SELECT id FROM tags WHERE id=?"
            };
            if sqlx::query_scalar::<_, String>(lock)
                .bind(tag_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(db_err)?
                .is_none()
            {
                continue;
            }
            let used: i64 =
                sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM taggings WHERE tag_id=?)")
                    .bind(tag_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db_err)?;
            if used != 0 {
                continue;
            }
            let filters = sqlx::query(
                "SELECT f.id,f.creator_id,f.fields FROM filters f \
                 JOIN filters_tags ft ON ft.filter_id=f.id WHERE ft.tag_id=? ORDER BY f.id",
            )
            .bind(tag_id)
            .fetch_all(&mut *tx)
            .await
            .map_err(db_err)?;
            for filter in filters {
                let filter_id: String = filter.get("id");
                let creator_id: String = filter.get("creator_id");
                let fields: String = filter.get("fields");
                let mut params: Value = serde_json::from_str(&fields)
                    .map_err(|_| err(StatusCode::INTERNAL_SERVER_ERROR, "Invalid saved filter"))?;
                let values = params
                    .get_mut("tag_ids")
                    .and_then(Value::as_array_mut)
                    .ok_or_else(|| {
                        err(
                            StatusCode::INTERNAL_SERVER_ERROR,
                            "Invalid saved filter tags",
                        )
                    })?;
                values.retain(|value| value.as_str() != Some(tag_id));
                if values.is_empty() {
                    params
                        .as_object_mut()
                        .expect("saved filter object")
                        .remove("tag_ids");
                }
                sqlx::query("DELETE FROM filters_tags WHERE filter_id=? AND tag_id=?")
                    .bind(&filter_id)
                    .bind(tag_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(db_err)?;
                let normalized =
                    filter_params(&HeaderMap::new(), &Bytes::from(params.to_string()))?;
                if normalized.as_object().is_none_or(serde_json::Map::is_empty) {
                    delete_filter(&mut tx, &filter_id).await?;
                } else {
                    let fields = normalized.to_string();
                    let digest = hash(&fields);
                    let duplicate: i64 = sqlx::query_scalar(
                        "SELECT EXISTS(SELECT 1 FROM filters WHERE creator_id=? AND params_digest=? AND id<>?)",
                    )
                    .bind(&creator_id)
                    .bind(&digest)
                    .bind(&filter_id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(db_err)?;
                    if duplicate != 0 {
                        delete_filter(&mut tx, &filter_id).await?;
                    } else {
                        sqlx::query(
                            "UPDATE filters SET fields=?,params_digest=?,updated_at=? WHERE id=?",
                        )
                        .bind(fields)
                        .bind(digest)
                        .bind(now())
                        .bind(&filter_id)
                        .execute(&mut *tx)
                        .await
                        .map_err(db_err)?;
                    }
                }
            }
            deleted += sqlx::query("DELETE FROM tags WHERE id=?")
                .bind(tag_id)
                .execute(&mut *tx)
                .await
                .map_err(db_err)?
                .rows_affected() as usize;
            tx.commit().await.map_err(db_err)?;
        }
        after = ids.last().expect("nonempty batch").clone();
    }
    Ok(deleted)
}

async fn claim_day(app: &App, day: &str) -> Result<bool> {
    let insert = if app.mysql {
        "INSERT IGNORE INTO app_secrets(name,value) VALUES('unused_tag_cleanup_day','0')"
    } else {
        "INSERT OR IGNORE INTO app_secrets(name,value) VALUES('unused_tag_cleanup_day','0')"
    };
    sqlx::query(insert).execute(&app.db).await.map_err(db_err)?;
    let result = sqlx::query(
        "UPDATE app_secrets SET value=? WHERE name='unused_tag_cleanup_day' AND value<?",
    )
    .bind(day)
    .bind(day)
    .execute(&app.db)
    .await
    .map_err(db_err)?;
    Ok(result.rows_affected() == 1)
}

pub(super) async fn worker(app: Arc<App>) {
    let mut interval = tokio::time::interval(Duration::from_secs(30));
    loop {
        interval.tick().await;
        let current = Utc::now();
        if current.hour() < 4 || (current.hour() == 4 && current.minute() < 2) {
            continue;
        }
        let day = current.format("%Y-%m-%d").to_string();
        match claim_day(&app, &day).await {
            Ok(true) => {
                if let Err(error) = delete_unused_tags(&app).await {
                    eprintln!("unused tag cleanup failed: {}", error.1);
                    if let Err(release) = sqlx::query("UPDATE app_secrets SET value='0' WHERE name='unused_tag_cleanup_day' AND value=?")
                        .bind(&day)
                        .execute(&app.db)
                        .await
                    {
                        eprintln!("unused tag cleanup claim release failed: {release}");
                    }
                }
            }
            Ok(false) => {}
            Err(error) => eprintln!("unused tag cleanup claim failed: {}", error.1),
        }
    }
}
