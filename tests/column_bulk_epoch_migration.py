#!/usr/bin/env python3
"""Check fresh and upgraded SQLite card-touch triggers."""

import os
import sqlite3
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/release/rizzy"
TRIGGERS = ("cards_touch_board_update", "cards_touch_column_update")
EPOCH_GUARD = "NEW.bulk_touch_epoch=OLD.bulk_touch_epoch"


def definitions(database):
    with sqlite3.connect(database) as db:
        return dict(db.execute("SELECT name,sql FROM sqlite_master WHERE type='trigger' AND name IN (?,?)", TRIGGERS))


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-column-epoch-migration-") as temporary:
        database = Path(temporary) / "rizzy.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env, check=True, capture_output=True)
        fresh = definitions(database)
        assert len(fresh) == len(TRIGGERS)
        assert all(EPOCH_GUARD in definition for definition in fresh.values())
        with sqlite3.connect(database) as db:
            for name in TRIGGERS:
                db.execute("DROP TRIGGER " + name)
                db.execute(fresh[name].replace(EPOCH_GUARD + " AND ", ""))
        legacy = definitions(database)
        assert all(EPOCH_GUARD not in definition for definition in legacy.values())
        subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env, check=True, capture_output=True)
        upgraded = definitions(database)
        assert all(EPOCH_GUARD in definition for definition in upgraded.values())
    print("SQLite card-touch triggers migrated to bulk epoch guard")


if __name__ == "__main__":
    main()
