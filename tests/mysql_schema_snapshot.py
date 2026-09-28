#!/usr/bin/env python3
"""Check that the MySQL DDL snapshot is reproducible from a fresh Rizzy DB."""

import difflib
import os
import subprocess
import tempfile
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-schema-snapshot-") as temporary:
        directory = Path(temporary)
        database = directory / "rizzy.sqlite3"
        generated = directory / "schema.mysql.sql"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}",
               "RIZZY_STORAGE_DIR": str(directory / "storage")}
        subprocess.run([ROOT / "target/release/rizzy", "seed"], cwd=ROOT,
                       env=env, stdout=subprocess.DEVNULL, check=True)
        subprocess.run(["python3", ROOT / "scripts/generate_mysql_schema.py",
                        database, generated], cwd=ROOT, check=True)
        expected = (ROOT / "schema.mysql.sql").read_text()
        actual = generated.read_text()
        if expected != actual:
            difference = difflib.unified_diff(expected.splitlines(True), actual.splitlines(True),
                                              fromfile="schema.mysql.sql", tofile="generated")
            raise AssertionError("MySQL schema snapshot differs:\n" + "".join(difference))
    print("PASS MySQL schema snapshot matches fresh SQLite schema")


if __name__ == "__main__":
    main()
