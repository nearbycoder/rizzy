#!/usr/bin/env python3
"""Restore bulk-touch table and guarded triggers in an existing MySQL schema."""

import os
import subprocess

from mysql_cluster import BINARY, ROOT, mysql_scalar


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL and RIZZY_TEST_MYSQL_CLIENT")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    env = {**os.environ, "RIZZY_DATABASE_URL": url}
    subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env, check=True)
    for name in ("cards_touch_board_update", "cards_touch_column_update"):
        mysql_scalar(url, f"DROP TRIGGER {name}")
    mysql_scalar(url, "DROP TABLE card_bulk_touches")
    mysql_scalar(url, "DELETE FROM app_secrets WHERE name='cards_bulk_touch_triggers_v2'")

    subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env, check=True)
    assert mysql_scalar(url,
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema=DATABASE() "
        "AND table_name='card_bulk_touches'") == "1"
    for name in ("cards_touch_board_update", "cards_touch_column_update"):
        definition = mysql_scalar(url,
            "SELECT ACTION_STATEMENT FROM information_schema.triggers "
            f"WHERE trigger_schema=DATABASE() AND trigger_name='{name}'")
        assert "card_bulk_touches" in definition, (name, definition)
    assert mysql_scalar(url,
        "SELECT COUNT(*) FROM app_secrets WHERE name='cards_bulk_touch_triggers_v2'") == "1"
    print("MySQL bulk-touch guard recovered on an existing schema")


if __name__ == "__main__":
    main()
