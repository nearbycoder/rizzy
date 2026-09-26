//! Remove polymorphic dependents before their owning rows disappear.
//!
//! MySQL foreign keys cannot target Card/Comment/Board records through the
//! record_type/record_id pair, and MySQL does not run child DELETE triggers
//! for foreign-key cascades. Keep this cleanup inside the caller's transaction.

use super::{Result, db_err};
use sqlx::{Any, Transaction};

const CARD_AND_COMMENT_TARGETS: [(&str, &str, &str); 3] = [
    ("rich_text_records", "record_type", "record_id"),
    ("mentions", "source_type", "source_id"),
    ("reactions", "reactable_type", "reactable_id"),
];

pub(super) async fn comment(
    tx: &mut Transaction<'_, Any>,
    account_id: &str,
    comment_id: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM notifications WHERE account_id=? AND source_type='Event' AND source_id IN (SELECT id FROM events WHERE eventable_type='Comment' AND eventable_id=?)")
        .bind(account_id)
        .bind(comment_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    sqlx::query("DELETE FROM notifications WHERE account_id=? AND source_type='Mention' AND source_id IN (SELECT id FROM mentions WHERE source_type='Comment' AND source_id=?)")
        .bind(account_id)
        .bind(comment_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    for (table, kind, key) in CARD_AND_COMMENT_TARGETS {
        let statement =
            format!("DELETE FROM {table} WHERE account_id=? AND {kind}='Comment' AND {key}=?");
        sqlx::query(&statement)
            .bind(account_id)
            .bind(comment_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}

pub(super) async fn card(
    tx: &mut Transaction<'_, Any>,
    account_id: &str,
    card_id: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM notifications WHERE account_id=? AND card_id=?")
        .bind(account_id)
        .bind(card_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    for (table, kind, key) in CARD_AND_COMMENT_TARGETS {
        let statement =
            format!("DELETE FROM {table} WHERE account_id=? AND {kind}='Card' AND {key}=?");
        sqlx::query(&statement)
            .bind(account_id)
            .bind(card_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        let statement = format!(
            "DELETE FROM {table} WHERE account_id=? AND {kind}='Comment' AND {key} IN (SELECT id FROM comments WHERE card_id=?)"
        );
        sqlx::query(&statement)
            .bind(account_id)
            .bind(card_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}

pub(super) async fn board(
    tx: &mut Transaction<'_, Any>,
    account_id: &str,
    board_id: &str,
) -> Result<()> {
    sqlx::query("DELETE FROM notifications WHERE account_id=? AND card_id IN (SELECT id FROM cards WHERE account_id=? AND board_id=?)")
        .bind(account_id)
        .bind(account_id)
        .bind(board_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    sqlx::query("DELETE FROM notifications WHERE account_id=? AND source_type='Event' AND source_id IN (SELECT id FROM events WHERE account_id=? AND board_id=?)")
        .bind(account_id)
        .bind(account_id)
        .bind(board_id)
        .execute(&mut **tx)
        .await
        .map_err(db_err)?;
    sqlx::query(
        "DELETE FROM rich_text_records WHERE account_id=? AND record_type='Board' AND record_id=?",
    )
    .bind(account_id)
    .bind(board_id)
    .execute(&mut **tx)
    .await
    .map_err(db_err)?;
    for (table, kind, key) in CARD_AND_COMMENT_TARGETS {
        let statement = format!(
            "DELETE FROM {table} WHERE account_id=? AND {kind}='Card' AND {key} IN (SELECT id FROM cards WHERE account_id=? AND board_id=?)"
        );
        sqlx::query(&statement)
            .bind(account_id)
            .bind(account_id)
            .bind(board_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
        let statement = format!(
            "DELETE FROM {table} WHERE account_id=? AND {kind}='Comment' AND {key} IN (SELECT m.id FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.account_id=? AND c.board_id=?)"
        );
        sqlx::query(&statement)
            .bind(account_id)
            .bind(account_id)
            .bind(board_id)
            .execute(&mut **tx)
            .await
            .map_err(db_err)?;
    }
    Ok(())
}
