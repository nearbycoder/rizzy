use super::*;

pub(super) async fn initialize(db: &DbPool, mysql: bool) -> sqlx::Result<()> {
    let mut tx = if mysql {
        db.begin().await?
    } else {
        db.begin_with("BEGIN IMMEDIATE").await?
    };
    let highest: i64 =
        sqlx::query_scalar("SELECT COALESCE(MAX(external_account_id),0) FROM accounts")
            .fetch_one(&mut *tx)
            .await?;
    if mysql {
        sqlx::query("INSERT INTO account_external_id_sequences(id,value) VALUES(1,?) ON DUPLICATE KEY UPDATE value=GREATEST(value,?)")
            .bind(highest)
            .bind(highest)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("INSERT INTO account_external_id_sequences(id,value) VALUES(1,?) ON CONFLICT(id) DO UPDATE SET value=MAX(value,excluded.value)")
            .bind(highest)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await
}

pub(super) async fn next(db: &DbPool, mysql: bool) -> sqlx::Result<i64> {
    let mut tx = if mysql {
        db.begin().await?
    } else {
        db.begin_with("BEGIN IMMEDIATE").await?
    };
    let updated = sqlx::query("UPDATE account_external_id_sequences SET value=value+1 WHERE id=1")
        .execute(&mut *tx)
        .await?;
    if updated.rows_affected() != 1 {
        return Err(sqlx::Error::RowNotFound);
    }
    let value = sqlx::query_scalar("SELECT value FROM account_external_id_sequences WHERE id=1")
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(value)
}
