use serde::Deserialize;
use sqlx::{Sqlite, Transaction};

use super::{id, rich_text, search_content};

#[derive(Deserialize)]
struct PlaygroundCard {
    title: String,
    description: String,
}

pub(super) async fn playground(
    tx: &mut Transaction<'_, Sqlite>,
    account_id: &str,
    owner_id: &str,
    created_at: &str,
) -> std::result::Result<(), sqlx::Error> {
    // Source: Account::Seeder at the pinned Fizzy revision. Keep this fixture
    // alongside the Rust binary so signup does not depend on the Rails tree.
    let cards: Vec<PlaygroundCard> =
        serde_json::from_str(include_str!("../static/signup_playground.json"))
            .expect("valid signup Playground fixture");
    let board_id = id();
    sqlx::query("INSERT INTO boards(id,account_id,creator_id,name,all_access,auto_postpone_period_in_days,entropy_override,created_at,updated_at) VALUES(?,?,?,'Playground',1,365,1,?,?)")
        .bind(&board_id).bind(account_id).bind(owner_id).bind(created_at).bind(created_at)
        .execute(&mut **tx).await?;
    sqlx::query("INSERT INTO accesses(board_id,user_id,involvement) VALUES(?,?,'watching')")
        .bind(&board_id)
        .bind(owner_id)
        .execute(&mut **tx)
        .await?;
    for (index, card) in cards.iter().enumerate() {
        let card_id = id();
        let search = search_content(rich_text::plain_text(&card.description));
        sqlx::query("INSERT INTO cards(id,account_id,board_id,creator_id,number,title,description,search_text,status,created_at,updated_at,last_active_at) VALUES(?,?,?,?,?,?,?,?,'published',?,?,?)")
            .bind(&card_id).bind(account_id).bind(&board_id).bind(owner_id)
            .bind((index + 1) as i64).bind(&card.title).bind(&card.description).bind(search)
            .bind(created_at).bind(created_at).bind(created_at).execute(&mut **tx).await?;
        sqlx::query("INSERT INTO watches(id,card_id,user_id,watching,created_at,updated_at) VALUES(?,?,?,1,?,?)")
            .bind(id())
            .bind(&card_id)
            .bind(owner_id)
            .bind(created_at)
            .bind(created_at)
            .execute(&mut **tx)
            .await?;
        sqlx::query("INSERT INTO events(id,account_id,board_id,card_id,creator_id,eventable_type,eventable_id,action,particulars,created_at) VALUES(?,?,?,?,?,'Card',?,'card_published','{}',?)")
            .bind(id()).bind(account_id).bind(&board_id).bind(&card_id).bind(owner_id)
            .bind(&card_id).bind(created_at).execute(&mut **tx).await?;
    }
    sqlx::query("UPDATE accounts SET cards_count=? WHERE id=?")
        .bind(cards.len() as i64)
        .bind(account_id)
        .execute(&mut **tx)
        .await?;
    Ok(())
}
