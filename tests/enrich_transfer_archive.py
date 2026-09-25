#!/usr/bin/env python3
"""Add valid Fizzy record types to the small account transfer fixture."""
import argparse
import json
import uuid
import zipfile
from datetime import datetime, timedelta


def new_id():
    return uuid.uuid4().hex[:25]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("source")
    parser.add_argument("output")
    args = parser.parse_args()
    with zipfile.ZipFile(args.source) as source:
        names = source.namelist()
        account = json.loads(source.read("data/account.json"))
        owner = next(json.loads(source.read(name)) for name in names if name.startswith("data/users/")
                     and json.loads(source.read(name))["role"] == "owner")
        card_path = next(name for name in names if name.startswith("data/cards/"))
        card = json.loads(source.read(card_path))
        blob_path = next(name for name in names if name.startswith("data/active_storage_blobs/"))
        blob = json.loads(source.read(blob_path))
        rich_path = next(name for name in names if name.startswith("data/action_text_rich_texts/")
                         and json.loads(source.read(name)).get("record_type") == "Card")
        rich = json.loads(source.read(rich_path))
        card["due_on"] = "2026-10-15"
        blob["metadata"] = {"analyzed": True, "width": 806, "height": 926}
        rich["body"] += (f'<p><action-text-attachment gid="gid://fizzy/User/{owner["id"]}" '
                         'content-type="application/vnd.actiontext.mention"></action-text-attachment></p>')
        created = card["created_at"]
        ending = (datetime.fromisoformat(created.replace("Z", "+00:00")) + timedelta(hours=1))
        ends_at = ending.isoformat(timespec="milliseconds").replace("+00:00", "Z")
        mention_id, notification_id, bundle_id, variant_id, attachment_id = (new_id() for _ in range(5))
        additions = {
            f"data/mentions/{mention_id}.json": {
                "id": mention_id, "account_id": account["id"], "mentionee_id": owner["id"],
                "mentioner_id": owner["id"], "source_type": "Card", "source_id": card["id"],
                "created_at": created, "updated_at": created,
            },
            f"data/notification_bundles/{bundle_id}.json": {
                "id": bundle_id, "account_id": account["id"], "user_id": owner["id"],
                "starts_at": created, "ends_at": ends_at, "status": "delivered",
                "created_at": created, "updated_at": created,
            },
            f"data/notifications/{notification_id}.json": {
                "id": notification_id, "account_id": account["id"], "card_id": card["id"],
                "creator_id": owner["id"], "user_id": owner["id"], "source_type": "Mention",
                "source_id": mention_id, "unread_count": 1, "read_at": None,
                "created_at": created, "updated_at": created,
            },
            f"data/active_storage_variant_records/{variant_id}.json": {
                "id": variant_id, "account_id": account["id"], "blob_id": blob["id"],
                "variation_digest": "extended-archive-variant-digest",
            },
            f"data/active_storage_attachments/{attachment_id}.json": {
                "id": attachment_id, "account_id": account["id"], "blob_id": blob["id"],
                "name": "image", "record_type": "ActiveStorage::VariantRecord",
                "record_id": variant_id, "created_at": created,
            },
        }
        with zipfile.ZipFile(args.output, "w") as output:
            for info in source.infolist():
                data = source.read(info.filename)
                if info.filename == card_path:
                    data = json.dumps(card).encode()
                elif info.filename == blob_path:
                    data = json.dumps(blob).encode()
                elif info.filename == rich_path:
                    data = json.dumps(rich).encode()
                output.writestr(info, data)
            for name, record in additions.items():
                output.writestr(name, json.dumps(record).encode(), compress_type=zipfile.ZIP_DEFLATED)
    print(f"Extended account archive written to {args.output}")


if __name__ == "__main__":
    main()
