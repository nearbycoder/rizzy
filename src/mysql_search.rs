//! MySQL's search index follows Fizzy's account-routed, stemmed FULLTEXT layout.

use crc32fast::hash;
use rust_stemmers::{Algorithm, Stemmer};
use sqlx::{Any, AnyPool, Row, Transaction};

pub(super) fn table(account_id: &str) -> String {
    format!("search_records_{}", hash(account_id.as_bytes()) % 16)
}

pub(super) fn stem_tokens(value: &str) -> Vec<String> {
    let stemmer = Stemmer::create(Algorithm::English);
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
        .filter(|word| !word.is_empty())
        .map(|word| stemmer.stem(&word.to_ascii_lowercase()).into_owned())
        .collect()
}

fn stem(value: &str) -> String {
    stem_tokens(value).join(" ")
}

pub(super) fn query(raw: &str, account_id: &str) -> Option<String> {
    let sanitized = super::fts_query(raw)?;
    let terms = stem(&sanitized);
    if terms.is_empty() {
        None
    } else {
        Some(format!("+account{account_id} +({terms})"))
    }
}

pub(super) async fn setup(db: &AnyPool) -> Result<(), Box<dyn std::error::Error>> {
    for shard in 0..16 {
        let table = format!("search_records_{shard}");
        let ddl = format!(
            "CREATE TABLE IF NOT EXISTS `{table}` (\
             `searchable_type` VARCHAR(16) NOT NULL,\
             `searchable_id` VARCHAR(64) NOT NULL,\
             `account_id` VARCHAR(64) NOT NULL,\
             `account_key` VARCHAR(128) NOT NULL,\
             `board_id` VARCHAR(64) NOT NULL,\
             `card_id` VARCHAR(64) NOT NULL,\
             `title` VARCHAR(255),\
             `content` LONGTEXT,\
             `created_at` VARCHAR(40) NOT NULL,\
             PRIMARY KEY (`searchable_type`,`searchable_id`),\
             KEY `search_account` (`account_id`),\
             FULLTEXT KEY `search_fulltext` (`account_key`,`content`,`title`),\
             CONSTRAINT `fk_search_records_{shard}_card` FOREIGN KEY (`card_id`) REFERENCES `cards` (`id`) ON DELETE CASCADE\
             ) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4 COLLATE=utf8mb4_bin"
        );
        sqlx::raw_sql(&ddl).execute(db).await?;
    }
    let indexes: Vec<String> = sqlx::query_scalar(
        "SELECT CONCAT(table_name,'.',index_name) FROM information_schema.statistics WHERE table_schema=DATABASE() AND table_name LIKE 'search_records_%'",
    )
    .fetch_all(db)
    .await?;
    let indexes: std::collections::HashSet<_> = indexes.into_iter().collect();
    for shard in 0..16 {
        for name in ["PRIMARY", "search_account", "search_fulltext"] {
            let key = format!("search_records_{shard}.{name}");
            if !indexes.contains(&key) {
                return Err(format!("incomplete MySQL search index: {key}").into());
            }
        }
    }
    let backfilled: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM app_secrets WHERE name='mysql_search_v1')")
            .fetch_one(db)
            .await?;
    if backfilled != 0 {
        return Ok(());
    }
    for (entity, statement) in [
        (
            "card",
            "SELECT id,account_id FROM cards WHERE status='published' AND id>? ORDER BY id LIMIT 200",
        ),
        (
            "comment",
            "SELECT m.id,c.account_id FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.status='published' AND m.id>? ORDER BY m.id LIMIT 200",
        ),
    ] {
        let mut after = String::new();
        loop {
            let rows = sqlx::query(statement).bind(&after).fetch_all(db).await?;
            if rows.is_empty() {
                break;
            }
            let mut tx = db.begin().await?;
            for row in &rows {
                let id: String = row.get("id");
                let account_id: String = row.get("account_id");
                if entity == "card" {
                    sync_card(&mut tx, &account_id, &id).await?;
                } else {
                    sync_comment(&mut tx, &account_id, &id).await?;
                }
                after = id;
            }
            tx.commit().await?;
        }
    }
    sqlx::query("INSERT INTO app_secrets(name,value) VALUES('mysql_search_v1','done')")
        .execute(db)
        .await?;
    Ok(())
}

pub(super) async fn sync_card(
    tx: &mut Transaction<'_, Any>,
    account_id: &str,
    card_id: &str,
) -> sqlx::Result<()> {
    let table = table(account_id);
    let row = sqlx::query("SELECT id,board_id,title,search_text,created_at,status FROM cards WHERE id=? AND account_id=?")
        .bind(card_id)
        .bind(account_id)
        .fetch_one(&mut **tx)
        .await?;
    if row.get::<String, _>("status") != "published" {
        remove(tx, account_id, "Card", card_id).await?;
        return Ok(());
    }
    let statement = format!(
        "INSERT INTO `{table}`(searchable_type,searchable_id,account_id,account_key,board_id,card_id,title,content,created_at) \
         VALUES('Card',?,?,?,?,?,?,?,?) ON DUPLICATE KEY UPDATE board_id=VALUES(board_id),title=VALUES(title),content=VALUES(content),created_at=VALUES(created_at)"
    );
    let title: String = row.get("title");
    let content: String = row.get("search_text");
    sqlx::query(&statement)
        .bind(card_id)
        .bind(account_id)
        .bind(format!("account{account_id}"))
        .bind(row.get::<String, _>("board_id"))
        .bind(card_id)
        .bind(stem(&title))
        .bind(stem(&content))
        .bind(row.get::<String, _>("created_at"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn sync_comment(
    tx: &mut Transaction<'_, Any>,
    account_id: &str,
    comment_id: &str,
) -> sqlx::Result<()> {
    let table = table(account_id);
    let row = sqlx::query("SELECT m.id,m.card_id,m.search_text,m.created_at,c.board_id,c.status FROM comments m JOIN cards c ON c.id=m.card_id WHERE m.id=? AND c.account_id=?")
        .bind(comment_id)
        .bind(account_id)
        .fetch_one(&mut **tx)
        .await?;
    if row.get::<String, _>("status") != "published" {
        remove(tx, account_id, "Comment", comment_id).await?;
        return Ok(());
    }
    let statement = format!(
        "INSERT INTO `{table}`(searchable_type,searchable_id,account_id,account_key,board_id,card_id,title,content,created_at) \
         VALUES('Comment',?,?,?,?,?,?,?,?) ON DUPLICATE KEY UPDATE board_id=VALUES(board_id),content=VALUES(content),created_at=VALUES(created_at)"
    );
    sqlx::query(&statement)
        .bind(comment_id)
        .bind(account_id)
        .bind(format!("account{account_id}"))
        .bind(row.get::<String, _>("board_id"))
        .bind(row.get::<String, _>("card_id"))
        .bind(None::<String>)
        .bind(stem(&row.get::<String, _>("search_text")))
        .bind(row.get::<String, _>("created_at"))
        .execute(&mut **tx)
        .await?;
    Ok(())
}

pub(super) async fn remove(
    tx: &mut Transaction<'_, Any>,
    account_id: &str,
    entity: &str,
    id: &str,
) -> sqlx::Result<()> {
    let statement = format!(
        "DELETE FROM `{}` WHERE searchable_type=? AND searchable_id=?",
        table(account_id)
    );
    sqlx::query(&statement)
        .bind(entity)
        .bind(id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{query, stem, table};

    #[test]
    fn source_mysql_stemming_examples() {
        assert_eq!(
            stem("test, running      JUMPING & walking"),
            "test run jump walk"
        );
        assert_eq!(stem("BC3-IOS-1D8B"), "bc3 io 1d8b");
        assert_eq!(
            query("authentication", "abc"),
            Some("+accountabc +(authent)".into())
        );
        assert_eq!(table("abc"), "search_records_2");
    }
}
