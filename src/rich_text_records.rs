//! Durable Action Text record identities for inline card, comment, and board text.
use super::{Result, db_err, id};
use sqlx::{QueryBuilder, Row, Sqlite, SqlitePool, Transaction};
use std::collections::HashSet;

pub(super) async fn present_ids(
    db: &SqlitePool,
    record_type: &str,
    name: &str,
    ids: &[String],
) -> Result<HashSet<String>> {
    if ids.is_empty() {
        return Ok(HashSet::new());
    }
    let mut query =
        QueryBuilder::<Sqlite>::new("SELECT record_id FROM rich_text_records WHERE record_type=");
    query
        .push_bind(record_type)
        .push(" AND body_is_null=0 AND name=")
        .push_bind(name)
        .push(" AND record_id IN (");
    let mut values = query.separated(",");
    for id in ids {
        values.push_bind(id);
    }
    values.push_unseparated(")");
    Ok(query
        .build_query_scalar()
        .fetch_all(db)
        .await
        .map_err(db_err)?
        .into_iter()
        .collect())
}

pub(super) async fn ensure(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
    record_type: &str,
    record_id: &str,
    name: &str,
    timestamp: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO rich_text_records(id,account_id,record_type,record_id,name,created_at,updated_at) VALUES(?,?,?,?,?,?,?) ON CONFLICT(record_type,record_id,name) DO UPDATE SET body_is_null=0,updated_at=excluded.updated_at WHERE rich_text_records.body_is_null=1")
        .bind(id()).bind(account_id).bind(record_type).bind(record_id).bind(name)
        .bind(timestamp).bind(timestamp)
        .execute(&mut **tx).await.map_err(db_err)?;
    Ok(())
}

pub(super) async fn ensure_null(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
    record_type: &str,
    record_id: &str,
    name: &str,
    timestamp: &str,
) -> Result<()> {
    sqlx::query("INSERT INTO rich_text_records(id,account_id,record_type,record_id,name,created_at,updated_at,body_is_null) VALUES(?,?,?,?,?,?,?,1) ON CONFLICT(record_type,record_id,name) DO UPDATE SET body_is_null=1,updated_at=excluded.updated_at WHERE rich_text_records.body_is_null=0")
        .bind(id()).bind(account_id).bind(record_type).bind(record_id).bind(name)
        .bind(timestamp).bind(timestamp)
        .execute(&mut **tx).await.map_err(db_err)?;
    Ok(())
}

pub(super) async fn sync(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
    record_type: &str,
    record_id: &str,
    name: &str,
    old_body: &str,
    new_body: &str,
    timestamp: &str,
) -> Result<()> {
    if old_body == new_body || (old_body.is_empty() && new_body.is_empty()) {
        return Ok(());
    }
    sqlx::query("INSERT INTO rich_text_records(id,account_id,record_type,record_id,name,created_at,updated_at) VALUES(?,?,?,?,?,?,?) ON CONFLICT(record_type,record_id,name) DO UPDATE SET body_is_null=0,updated_at=excluded.updated_at")
        .bind(id()).bind(account_id).bind(record_type).bind(record_id).bind(name)
        .bind(timestamp).bind(timestamp)
        .execute(&mut **tx).await.map_err(db_err)?;
    Ok(())
}

pub(super) async fn migrate(db: &SqlitePool) -> std::result::Result<(), sqlx::Error> {
    let columns = sqlx::query("PRAGMA table_info(rich_text_records)")
        .fetch_all(db)
        .await?;
    if !columns
        .iter()
        .any(|column| column.get::<String, _>("name") == "body_is_null")
    {
        sqlx::query(
            "ALTER TABLE rich_text_records ADD COLUMN body_is_null INTEGER NOT NULL DEFAULT 0",
        )
        .execute(db)
        .await?;
    }
    Ok(())
}

pub(super) async fn backfill(db: &SqlitePool) -> std::result::Result<(), sqlx::Error> {
    let sources = [
        (
            "Card",
            "description",
            "SELECT c.id,c.account_id,c.created_at,c.updated_at FROM cards c WHERE c.description!='' AND NOT EXISTS(SELECT 1 FROM rich_text_records r WHERE r.record_type='Card' AND r.record_id=c.id AND r.name='description')",
        ),
        (
            "Comment",
            "body",
            "SELECT m.id,c.account_id,m.created_at,COALESCE(m.updated_at,m.created_at) updated_at FROM comments m JOIN cards c ON c.id=m.card_id WHERE m.body!='' AND NOT EXISTS(SELECT 1 FROM rich_text_records r WHERE r.record_type='Comment' AND r.record_id=m.id AND r.name='body')",
        ),
        (
            "Board",
            "description",
            "SELECT b.id,b.account_id,b.created_at,b.updated_at FROM boards b WHERE b.public_description!='' AND NOT EXISTS(SELECT 1 FROM rich_text_records r WHERE r.record_type='Board' AND r.record_id=b.id AND r.name='description')",
        ),
    ];
    for (record_type, name, query) in sources {
        let rows = sqlx::query(query).fetch_all(db).await?;
        if rows.is_empty() {
            continue;
        }
        let mut tx = db.begin().await?;
        for row in rows {
            sqlx::query("INSERT OR IGNORE INTO rich_text_records(id,account_id,record_type,record_id,name,created_at,updated_at) VALUES(?,?,?,?,?,?,?)")
                .bind(id())
                .bind(row.get::<String, _>("account_id"))
                .bind(record_type)
                .bind(row.get::<String, _>("id"))
                .bind(name)
                .bind(row.get::<String, _>("created_at"))
                .bind(row.get::<String, _>("updated_at"))
                .execute(&mut *tx).await?;
        }
        tx.commit().await?;
    }
    Ok(())
}
