use std::{
    collections::{HashMap, HashSet},
    convert::Infallible,
    sync::{Arc, OnceLock},
    time::Duration,
};

use axum::{
    extract::{Path, State},
    http::HeaderMap,
    response::sse::{Event, KeepAlive, Sse},
};
use futures_util::stream;
use sqlx::{QueryBuilder, Row};
use tokio::sync::{Mutex, watch};

use super::{App, Db, DbPool, Result, Shared, card, db_err, principal};

struct LiveCard {
    revisions: watch::Sender<String>,
}

#[derive(Default)]
struct LiveRegistry {
    cards: HashMap<String, Arc<LiveCard>>,
    running: bool,
}

fn registry() -> &'static Mutex<LiveRegistry> {
    static LIVE_CARDS: OnceLock<Mutex<LiveRegistry>> = OnceLock::new();
    LIVE_CARDS.get_or_init(|| Mutex::new(LiveRegistry::default()))
}

async fn revision(db: &DbPool, card_id: &str) -> std::result::Result<Option<String>, sqlx::Error> {
    let row = sqlx::query("SELECT c.updated_at,(SELECT MAX(id) FROM comments WHERE card_id=c.id) AS latest_comment_id FROM cards c WHERE c.id=?")
        .bind(card_id)
        .fetch_optional(db)
        .await?;
    Ok(row.map(|row| {
        format!(
            "{}:{}",
            row.get::<String, _>("updated_at"),
            row.get::<Option<String>, _>("latest_comment_id")
                .unwrap_or_default()
        )
    }))
}

fn poll(app: Arc<App>) {
    tokio::spawn(async move {
        let mut ticks = tokio::time::interval(Duration::from_millis(1500));
        ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            ticks.tick().await;
            let active = {
                let mut registry = registry().lock().await;
                registry
                    .cards
                    .retain(|_, card| card.revisions.receiver_count() > 0);
                if registry.cards.is_empty() {
                    registry.running = false;
                    return;
                }
                registry.cards.clone()
            };
            let ids = active.keys().collect::<Vec<_>>();
            for chunk in ids.chunks(200) {
                let mut query = QueryBuilder::<Db>::new(
                    "SELECT c.id,c.updated_at,(SELECT MAX(id) FROM comments WHERE card_id=c.id) AS latest_comment_id FROM cards c WHERE c.id IN (",
                );
                let mut separated = query.separated(",");
                for id in chunk {
                    separated.push_bind(*id);
                }
                separated.push_unseparated(")");
                match query.build().fetch_all(&app.db).await {
                    Ok(rows) => {
                        let mut found = HashSet::new();
                        for row in rows {
                            let id: String = row.get("id");
                            found.insert(id.clone());
                            let current = format!(
                                "{}:{}",
                                row.get::<String, _>("updated_at"),
                                row.get::<Option<String>, _>("latest_comment_id")
                                    .unwrap_or_default()
                            );
                            if let Some(card) = active.get(&id) {
                                if *card.revisions.borrow() != current {
                                    let _ = card.revisions.send(current);
                                }
                            }
                        }
                        if found.len() < chunk.len() {
                            let mut registry = registry().lock().await;
                            for id in chunk {
                                if !found.contains(*id)
                                    && registry.cards.get(*id).is_some_and(|current| {
                                        active.get(*id).is_some_and(|old| Arc::ptr_eq(current, old))
                                    })
                                {
                                    registry.cards.remove(*id);
                                }
                            }
                        }
                    }
                    Err(error) => eprintln!("Card refresh probe failed: {error}"),
                }
            }
        }
    });
}

pub(super) async fn stream(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>>> {
    let principal = principal(&app, &headers, &account).await?;
    let card = card(&app, &principal, &number).await?;
    let card_id: String = card.get("id");
    let current = revision(&app.db, &card_id)
        .await
        .map_err(db_err)?
        .unwrap_or_default();
    let (receiver, start_poller) = {
        let mut registry = registry().lock().await;
        let receiver = if let Some(live) = registry.cards.get(&card_id) {
            live.revisions.subscribe()
        } else {
            let (revisions, receiver) = watch::channel(current);
            registry
                .cards
                .insert(card_id, Arc::new(LiveCard { revisions }));
            receiver
        };
        let start_poller = !registry.running;
        registry.running = true;
        (receiver, start_poller)
    };
    if start_poller {
        poll(app);
    }
    let events = stream::unfold((receiver, true), |(mut receiver, initial)| async move {
        if !initial && receiver.changed().await.is_err() {
            return None;
        }
        let value = receiver.borrow().clone();
        Some((Ok(Event::default().data(value)), (receiver, false)))
    });
    Ok(Sse::new(events).keep_alive(KeepAlive::default()))
}

pub(super) async fn current(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<axum::Json<serde_json::Value>> {
    let principal = principal(&app, &headers, &account).await?;
    let card = card(&app, &principal, &number).await?;
    let card_id: String = card.get("id");
    let value = revision(&app.db, &card_id)
        .await
        .map_err(db_err)?
        .unwrap_or_default();
    Ok(axum::Json(serde_json::json!({"revision":value})))
}
