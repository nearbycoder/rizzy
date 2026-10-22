#!/usr/bin/env python3
"""Upgrade old Tag/Tagging rows without losing their record identity."""

import os
import sqlite3
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-tag-record-migration-") as temporary:
        database = Path(temporary) / "rizzy.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env, check=True, capture_output=True)
        with sqlite3.connect(database) as db:
            db.execute("DROP TRIGGER tags_updated_at_insert")
            db.execute("DROP TRIGGER taggings_record_insert")
            db.execute("DROP INDEX taggings_id")
            db.execute("ALTER TABLE tags DROP COLUMN updated_at")
            for column in ("id", "created_at", "updated_at"):
                db.execute("ALTER TABLE taggings DROP COLUMN " + column)
            db.execute("DELETE FROM app_secrets WHERE name='tag_records_v1'")
            db.execute("INSERT INTO tags(id,account_id,title,created_at) VALUES('old-tag','old-account','old','2020-01-01T00:00:00.000Z')")
            db.execute("INSERT INTO taggings(card_id,tag_id) VALUES('old-card','old-tag')")
        subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env, check=True, capture_output=True)
        with sqlite3.connect(database) as db:
            assert db.execute("SELECT created_at,updated_at FROM tags WHERE id='old-tag'").fetchone() == (
                "2020-01-01T00:00:00.000Z", "2020-01-01T00:00:00.000Z")
            first = db.execute("SELECT id,created_at,updated_at FROM taggings WHERE card_id='old-card'").fetchone()
            assert first and first[0] and first[1] and first[2] == first[1]
            db.execute("INSERT INTO taggings(card_id,tag_id) VALUES('another-card','old-tag')")
            second = db.execute("SELECT id,created_at,updated_at FROM taggings WHERE card_id='another-card'").fetchone()
            assert second and second[0] and second[0] != first[0] and second[2] == second[1]
    print("Legacy Tag and Tagging rows migrated with stable IDs and timestamps")


if __name__ == "__main__":
    main()
