#!/usr/bin/env python3
"""Upgrade old Pin and Reaction rows with persistent update times."""

import os
import sqlite3
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"
CREATED = "2020-01-01T00:00:00.000Z"


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-interaction-record-migration-") as temporary:
        database = Path(temporary) / "rizzy.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env, check=True, capture_output=True)
        with sqlite3.connect(database) as db:
            for table in ("pins", "reactions"):
                db.execute("DROP TRIGGER " + table + "_updated_at_insert")
                db.execute("ALTER TABLE " + table + " DROP COLUMN updated_at")
            db.execute("DELETE FROM app_secrets WHERE name='pin_reaction_update_v1'")
            db.execute("INSERT INTO pins(id,account_id,card_id,user_id,created_at) VALUES('pin-old','account','card','user',?)", (CREATED,))
            db.execute("INSERT INTO reactions(id,account_id,reactable_type,reactable_id,reacter_id,content,created_at) VALUES('reaction-old','account','Card','card','user','👍',?)", (CREATED,))
        subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env, check=True, capture_output=True)
        with sqlite3.connect(database) as db:
            for table, record_id in (("pins", "pin-old"), ("reactions", "reaction-old")):
                assert db.execute(f"SELECT created_at,updated_at FROM {table} WHERE id=?", (record_id,)).fetchone() == (CREATED, CREATED)
            db.execute("INSERT INTO pins(id,account_id,card_id,user_id,created_at) VALUES('pin-new','account','card-new','user',?)", (CREATED,))
            db.execute("INSERT INTO reactions(id,account_id,reactable_type,reactable_id,reacter_id,content,created_at) VALUES('reaction-new','account','Card','card','user','👍',?)", (CREATED,))
            for table, record_id in (("pins", "pin-new"), ("reactions", "reaction-new")):
                assert db.execute(f"SELECT created_at,updated_at FROM {table} WHERE id=?", (record_id,)).fetchone() == (CREATED, CREATED)
    print("Legacy Pin and Reaction rows migrated with stored update times")


if __name__ == "__main__":
    main()
