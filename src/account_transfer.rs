//! Fizzy's account transfer ZIP layout for records represented in Rizzy.
//! The generated archive covers Rizzy's current records and the tested Fizzy
//! import workflow; it does not yet contain every Fizzy record type.
use super::*;
use crate::exports::{ArchiveEntry, export_path, write_archive};
use futures_util::TryStreamExt;
use sqlx::{Column, TypeInfo, ValueRef};
use std::sync::OnceLock;

type BuildResult<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;

struct RecordSet {
    directory: &'static str,
    query: &'static str,
    booleans: &'static [&'static str],
    json_fields: &'static [&'static str],
}

// Keep this order aligned with Account::DataTransfer::Manifest in Fizzy.
const RECORD_SETS: &[RecordSet] = &[
    RecordSet {
        directory: "users",
        query: "SELECT u.id,u.account_id,u.active,u.created_at,i.email_address,u.identity_id,u.name,u.role,u.created_at updated_at,u.verified_at FROM users u LEFT JOIN identities i ON i.id=u.identity_id WHERE u.account_id=? ORDER BY u.id",
        booleans: &["active"],
        json_fields: &[],
    },
    RecordSet {
        directory: "user_settings",
        query: "SELECT NULL id,u.account_id,COALESCE(n.bundle_email_frequency,'never') bundle_email_frequency,u.created_at,COALESCE(t.timezone_name,NULL) timezone_name,u.created_at updated_at,u.id user_id FROM users u LEFT JOIN notification_settings n ON n.user_id=u.id LEFT JOIN user_timezones t ON t.user_id=u.id WHERE u.account_id=? AND u.role!='system' ORDER BY u.id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "tags",
        query: "SELECT id,account_id,created_at,title,created_at updated_at FROM tags WHERE account_id=? ORDER BY id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "boards",
        query: "SELECT id,account_id,all_access,created_at,creator_id,name,updated_at FROM boards WHERE account_id=? ORDER BY id",
        booleans: &["all_access"],
        json_fields: &[],
    },
    RecordSet {
        directory: "columns",
        query: "SELECT id,account_id,board_id,color,created_at,name,position,updated_at FROM columns WHERE account_id=? ORDER BY id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "entropies",
        query: "SELECT NULL id,a.id account_id,a.auto_postpone_period_in_days*86400 auto_postpone_period,a.id container_id,'Account' container_type,a.created_at,a.created_at updated_at FROM accounts a WHERE a.id=? UNION ALL SELECT NULL,b.account_id,b.auto_postpone_period_in_days*86400,b.id,'Board',b.created_at,b.updated_at FROM boards b WHERE b.account_id=? AND b.entropy_override=1",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "board_publications",
        query: "SELECT NULL id,account_id,id board_id,created_at,publication_key key,updated_at FROM boards WHERE account_id=? AND publication_key IS NOT NULL ORDER BY id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "webhooks",
        query: "SELECT id,account_id,active,board_id,created_at,name,signing_secret,subscribed_actions,updated_at,url FROM webhooks WHERE account_id=? ORDER BY id",
        booleans: &["active"],
        json_fields: &["subscribed_actions"],
    },
    RecordSet {
        directory: "accesses",
        query: "SELECT NULL id,x.accessed_at,b.account_id,x.board_id,COALESCE(x.accessed_at,b.created_at) created_at,x.involvement,COALESCE(x.accessed_at,b.created_at) updated_at,x.user_id FROM accesses x JOIN boards b ON b.id=x.board_id WHERE b.account_id=? ORDER BY x.board_id,x.user_id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "cards",
        query: "SELECT id,account_id,board_id,column_id,created_at,creator_id,NULL due_on,last_active_at,number,status,title,updated_at FROM cards WHERE account_id=? ORDER BY id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "comments",
        query: "SELECT m.id,c.account_id,m.card_id,m.created_at,m.creator_id,COALESCE(m.updated_at,m.created_at) updated_at FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.account_id=? ORDER BY m.id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "steps",
        query: "SELECT s.id,c.account_id,s.card_id,s.completed,s.content,s.created_at,s.created_at updated_at FROM steps s JOIN cards c ON c.id=s.card_id WHERE c.account_id=? ORDER BY s.id",
        booleans: &["completed"],
        json_fields: &[],
    },
    RecordSet {
        directory: "assignments",
        query: "SELECT id,account_id,assignee_id,assigner_id,card_id,created_at,created_at updated_at FROM assignments WHERE account_id=? ORDER BY id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "taggings",
        query: "SELECT NULL id,c.account_id,t.card_id,c.created_at,t.tag_id,c.created_at updated_at FROM taggings t JOIN cards c ON c.id=t.card_id WHERE c.account_id=? ORDER BY t.card_id,t.tag_id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "closures",
        query: "SELECT NULL id,c.account_id,z.card_id,z.created_at,z.created_at updated_at,z.user_id FROM closures z JOIN cards c ON c.id=z.card_id WHERE c.account_id=? ORDER BY z.card_id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "card_goldnesses",
        query: "SELECT NULL id,account_id,id card_id,updated_at created_at,updated_at FROM cards WHERE account_id=? AND golden=1 ORDER BY id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "card_not_nows",
        query: "SELECT NULL id,c.account_id,c.id card_id,c.updated_at created_at,c.updated_at,COALESCE((SELECT e.creator_id FROM events e WHERE e.card_id=c.id AND e.action='card_postponed' ORDER BY e.created_at DESC LIMIT 1),c.creator_id) user_id FROM cards c WHERE c.account_id=? AND c.postponed=1 ORDER BY c.id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "card_activity_spikes",
        query: "SELECT NULL id,account_id,card_id,created_at,updated_at FROM card_activity_spikes WHERE account_id=? ORDER BY card_id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "watches",
        query: "SELECT NULL id,c.account_id,w.card_id,c.created_at,c.created_at updated_at,w.user_id,w.watching FROM watches w JOIN cards c ON c.id=w.card_id WHERE c.account_id=? ORDER BY w.card_id,w.user_id",
        booleans: &["watching"],
        json_fields: &[],
    },
    RecordSet {
        directory: "pins",
        query: "SELECT id,account_id,card_id,created_at,created_at updated_at,user_id FROM pins WHERE account_id=? ORDER BY id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "reactions",
        query: "SELECT id,account_id,content,created_at,reactable_id,reactable_type,reacter_id,created_at updated_at FROM reactions WHERE account_id=? ORDER BY id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "filters",
        query: "SELECT id,account_id,created_at,creator_id,fields,params_digest,updated_at FROM filters WHERE account_id=? ORDER BY id",
        booleans: &[],
        json_fields: &["fields"],
    },
    RecordSet {
        directory: "webhook_delinquency_trackers",
        query: "SELECT NULL id,account_id,consecutive_failures_count,created_at,first_failure_at,updated_at,id webhook_id FROM webhooks WHERE account_id=? AND consecutive_failures_count>0 ORDER BY id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "events",
        query: "SELECT id,account_id,action,board_id,created_at,creator_id,eventable_id,eventable_type,particulars,created_at updated_at FROM events WHERE account_id=? ORDER BY id",
        booleans: &[],
        json_fields: &["particulars"],
    },
    RecordSet {
        directory: "notifications",
        query: "SELECT id,account_id,card_id,created_at,creator_id,read_at,source_id,source_type,unread_count,updated_at,user_id FROM notifications WHERE account_id=? ORDER BY id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "webhook_deliveries",
        query: "SELECT id,account_id,created_at,event_id,request,response,state,updated_at,webhook_id FROM webhook_deliveries WHERE account_id=? ORDER BY id",
        booleans: &[],
        json_fields: &["request", "response"],
    },
    RecordSet {
        directory: "active_storage_blobs",
        query: "SELECT id,account_id,byte_size,checksum,content_type,created_at,filename,storage_key key,'{}' metadata FROM storage_blobs WHERE account_id=? AND uploaded_at IS NOT NULL ORDER BY id",
        booleans: &[],
        json_fields: &["metadata"],
    },
    RecordSet {
        directory: "active_storage_attachments",
        query: "SELECT id,account_id,blob_id,created_at,name,record_id,record_type FROM storage_attachments WHERE account_id=? ORDER BY id",
        booleans: &[],
        json_fields: &[],
    },
    RecordSet {
        directory: "action_text_rich_texts",
        query: "SELECT NULL id,account_id,description body,created_at,'description' name,id record_id,'Card' record_type,updated_at FROM cards WHERE account_id=? AND description!='' UNION ALL SELECT NULL,c.account_id,m.body,m.created_at,'body',m.id,'Comment',COALESCE(m.updated_at,m.created_at) FROM comments m JOIN cards c ON c.id=m.card_id WHERE c.account_id=? AND m.body!='' UNION ALL SELECT NULL,account_id,public_description,created_at,'description',id,'Board',updated_at FROM boards WHERE account_id=? AND public_description!=''",
        booleans: &[],
        json_fields: &[],
    },
];

fn row_value(row: &SqliteRow, record_set: &RecordSet) -> BuildResult<Value> {
    let mut object = serde_json::Map::new();
    for (index, column) in row.columns().iter().enumerate() {
        let name = column.name();
        let raw = row.try_get_raw(index)?;
        let value = if raw.is_null() {
            Value::Null
        } else if record_set.booleans.contains(&name) {
            json!(row.try_get::<i64, _>(index)? != 0)
        } else if record_set.json_fields.contains(&name) {
            serde_json::from_str(&row.try_get::<String, _>(index)?)?
        } else {
            match raw.type_info().name() {
                "INTEGER" => json!(row.try_get::<i64, _>(index)?),
                "REAL" => json!(row.try_get::<f64, _>(index)?),
                _ => json!(row.try_get::<String, _>(index)?),
            }
        };
        object.insert(name.to_owned(), value);
    }
    if object.get("id").is_none_or(Value::is_null) {
        object.insert("id".into(), json!(id()));
    }
    if record_set.directory == "users"
        && object.get("role").and_then(Value::as_str) == Some("system")
    {
        object.insert("email_address".into(), Value::Null);
        object.insert("identity_id".into(), Value::Null);
    }
    Ok(Value::Object(object))
}

fn portable_rich_text(raw: &str, blobs: &HashMap<String, String>) -> String {
    static TAG: OnceLock<regex::Regex> = OnceLock::new();
    static SGID: OnceLock<regex::Regex> = OnceLock::new();
    let tag = TAG.get_or_init(|| {
        regex::Regex::new(r"(?is)<action-text-attachment\b[^>]*>").expect("attachment tag")
    });
    let sgid = SGID.get_or_init(|| {
        regex::Regex::new(r#"(?i)\bsgid\s*=\s*["']([^"']+)["']"#).expect("attachment sgid")
    });
    tag.replace_all(raw, |captures: &regex::Captures<'_>| {
        let original = captures.get(0).expect("attachment tag").as_str();
        let Some(value) = sgid.captures(original) else {
            return original.to_owned();
        };
        let Some(blob_id) = value.get(1).and_then(|value| blobs.get(value.as_str())) else {
            return original.to_owned();
        };
        sgid.replace(
            original,
            format!("gid=\"gid://fizzy/ActiveStorage::Blob/{blob_id}\""),
        )
        .into_owned()
    })
    .into_owned()
}

async fn emit_record_set(
    app: &App,
    sender: &tokio::sync::mpsc::Sender<ArchiveEntry>,
    account_id: &str,
    record_set: &RecordSet,
    blobs: &HashMap<String, String>,
) -> BuildResult<()> {
    let mut query = sqlx::query(record_set.query);
    for _ in 0..record_set.query.matches('?').count() {
        query = query.bind(account_id);
    }
    let mut rows = query.fetch(&app.db);
    while let Some(row) = rows.try_next().await? {
        let mut value = row_value(&row, record_set)?;
        if record_set.directory == "action_text_rich_texts" {
            if let Some(body) = value["body"].as_str() {
                value["body"] = json!(portable_rich_text(body, blobs));
            }
        }
        let record_id = value["id"].as_str().ok_or("missing record ID")?;
        sender
            .send(ArchiveEntry::Json(
                format!("data/{}/{record_id}.json", record_set.directory),
                serde_json::to_vec(&value)?,
            ))
            .await?;
    }
    Ok(())
}

async fn emit_account(
    app: &App,
    sender: &tokio::sync::mpsc::Sender<ArchiveEntry>,
    account_id: &str,
) -> BuildResult<()> {
    let row = sqlx::query("SELECT a.*,j.id join_id,j.code,j.usage_count,j.usage_limit,j.created_at join_created_at,j.updated_at join_updated_at FROM accounts a JOIN account_join_codes j ON j.account_id=a.id WHERE a.id=?")
        .bind(account_id).fetch_one(&app.db).await?;
    let created: String = row.get("created_at");
    let value = json!({
        "id":account_id,
        "cards_count":row.get::<i64,_>("cards_count"),
        "created_at":created,
        "external_account_id":row.get::<i64,_>("external_account_id"),
        "name":row.get::<String,_>("name"),
        "updated_at":created,
        "join_code":{
            "id":row.get::<String,_>("join_id"),
            "account_id":account_id,
            "code":row.get::<String,_>("code"),
            "usage_count":row.get::<i64,_>("usage_count"),
            "usage_limit":row.get::<i64,_>("usage_limit"),
            "created_at":row.get::<String,_>("join_created_at"),
            "updated_at":row.get::<String,_>("join_updated_at")
        }
    });
    sender
        .send(ArchiveEntry::Json(
            "data/account.json".into(),
            serde_json::to_vec(&value)?,
        ))
        .await?;
    Ok(())
}

pub(crate) async fn build(app: &App, export: &SqliteRow) -> BuildResult<()> {
    let export_id: String = export.get("id");
    let account_id: String = export.get("account_id");
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    let path = export_path(&export_id);
    let writer = tokio::task::spawn_blocking(move || write_archive(path, receiver));
    let result = async {
        emit_account(app, &sender, &account_id).await?;
        let blob_rows = sqlx::query("SELECT id,attachable_sgid FROM storage_blobs WHERE account_id=?")
            .bind(&account_id).fetch_all(&app.db).await?;
        let blob_ids: HashMap<String, String> = blob_rows.iter().map(|row| {
            (row.get("attachable_sgid"), row.get("id"))
        }).collect();
        for record_set in RECORD_SETS {
            emit_record_set(app, &sender, &account_id, record_set, &blob_ids).await?;
            sqlx::query("UPDATE exports SET updated_at=? WHERE id=?")
                .bind(now()).bind(&export_id).execute(&app.db).await?;
        }
        let mut blobs = sqlx::query("SELECT storage_key FROM storage_blobs WHERE account_id=? AND uploaded_at IS NOT NULL ORDER BY id")
            .bind(&account_id).fetch(&app.db);
        while let Some(blob) = blobs.try_next().await? {
            let key: String = blob.get("storage_key");
            sender.send(ArchiveEntry::Blob(format!("storage/{key}"), storage::storage_dir().join(key))).await?;
        }
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    }.await;
    drop(sender);
    let archive_result = writer.await?;
    if let Err(error) = result {
        let _ = tokio::fs::remove_file(export_path(&export_id)).await;
        return Err(error);
    }
    archive_result?;
    Ok(())
}
