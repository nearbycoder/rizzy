use std::{
    collections::HashMap,
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
use sha2::{Digest, Sha256};
use sqlx::{QueryBuilder, Row};
use tokio::sync::{Mutex, watch};

use super::{App, Db, DbPool, Result, Shared, db_err, principal};

struct LiveUser {
    revisions: watch::Sender<String>,
}

#[derive(Default)]
struct LiveRegistry {
    users: HashMap<String, Arc<LiveUser>>,
    running: bool,
}

fn registry() -> &'static Mutex<LiveRegistry> {
    static LIVE_USERS: OnceLock<Mutex<LiveRegistry>> = OnceLock::new();
    LIVE_USERS.get_or_init(|| Mutex::new(LiveRegistry::default()))
}

#[derive(Default)]
struct RevisionParts {
    notifications: Sha256,
    pins: Sha256,
}

fn add_field(hash: &mut Sha256, value: &str) {
    hash.update((value.len() as u64).to_le_bytes());
    hash.update(value.as_bytes());
}

async fn snapshots(
    db: &DbPool,
    mysql: bool,
    ids: &[String],
) -> std::result::Result<HashMap<String, String>, sqlx::Error> {
    let mut parts: HashMap<String, RevisionParts> = ids
        .iter()
        .map(|id| (id.clone(), RevisionParts::default()))
        .collect();
    let modified = if mysql {
        "CAST(n.updated_at AS CHAR)"
    } else {
        "n.updated_at"
    };
    let card_modified = if mysql {
        "CAST(c.updated_at AS CHAR)"
    } else {
        "c.updated_at"
    };
    let read_at = if mysql {
        "CAST(n.read_at AS CHAR)"
    } else {
        "n.read_at"
    };
    let mut query = QueryBuilder::<Db>::new(format!(
        "SELECT n.user_id,n.id,n.source_id,n.unread_count,{modified} AS modified,{read_at} AS read_at,{card_modified} AS card_modified FROM notifications n JOIN cards c ON c.id=n.card_id WHERE n.user_id IN ("
    ));
    let mut separated = query.separated(",");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(") ORDER BY n.user_id,n.id");
    for row in query.build().fetch_all(db).await? {
        let id: String = row.get("user_id");
        if let Some(part) = parts.get_mut(&id) {
            let hash = &mut part.notifications;
            add_field(hash, &row.get::<String, _>("id"));
            add_field(hash, &row.get::<String, _>("source_id"));
            add_field(hash, &row.get::<i64, _>("unread_count").to_string());
            add_field(hash, &row.get::<String, _>("modified"));
            add_field(
                hash,
                &row.get::<Option<String>, _>("read_at").unwrap_or_default(),
            );
            add_field(hash, &row.get::<String, _>("card_modified"));
        }
    }
    let pin_modified = if mysql {
        "CAST(p.updated_at AS CHAR)"
    } else {
        "p.updated_at"
    };
    let mut query = QueryBuilder::<Db>::new(format!(
        "SELECT p.user_id,p.id,{pin_modified} AS modified,{card_modified} AS card_modified FROM pins p JOIN cards c ON c.id=p.card_id WHERE p.user_id IN ("
    ));
    let mut separated = query.separated(",");
    for id in ids {
        separated.push_bind(id);
    }
    separated.push_unseparated(") ORDER BY p.user_id,p.id");
    for row in query.build().fetch_all(db).await? {
        let id: String = row.get("user_id");
        if let Some(part) = parts.get_mut(&id) {
            let hash = &mut part.pins;
            add_field(hash, &row.get::<String, _>("id"));
            add_field(hash, &row.get::<String, _>("modified"));
            add_field(hash, &row.get::<String, _>("card_modified"));
        }
    }
    Ok(parts
        .into_iter()
        .map(|(id, part)| {
            let notifications = format!("{:x}", part.notifications.finalize());
            let pins = format!("{:x}", part.pins.finalize());
            (
                id,
                serde_json::json!({ "notifications": notifications, "pins": pins }).to_string(),
            )
        })
        .collect())
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
                    .users
                    .retain(|_, user| user.revisions.receiver_count() > 0);
                if registry.users.is_empty() {
                    registry.running = false;
                    return;
                }
                registry.users.clone()
            };
            let ids = active.keys().cloned().collect::<Vec<_>>();
            for chunk in ids.chunks(200) {
                match snapshots(&app.db, app.mysql, chunk).await {
                    Ok(values) => {
                        for (id, revision) in values {
                            if let Some(user) = active.get(&id) {
                                if *user.revisions.borrow() != revision {
                                    let _ = user.revisions.send(revision);
                                }
                            }
                        }
                    }
                    Err(error) => eprintln!("User tray refresh probe failed: {error}"),
                }
            }
        }
    });
}

pub(super) async fn stream(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
) -> Result<Sse<impl futures_util::Stream<Item = std::result::Result<Event, Infallible>>>> {
    let user = principal(&app, &headers, &account).await?;
    let user_id = user.user_id;
    let initial = snapshots(&app.db, app.mysql, std::slice::from_ref(&user_id))
        .await
        .map_err(db_err)?
        .remove(&user_id)
        .unwrap_or_default();
    let (receiver, start_poller) = {
        let mut registry = registry().lock().await;
        let receiver = if let Some(live) = registry.users.get(&user_id) {
            live.revisions.subscribe()
        } else {
            let (revisions, receiver) = watch::channel(initial);
            registry
                .users
                .insert(user_id, Arc::new(LiveUser { revisions }));
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
