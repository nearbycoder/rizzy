#!/usr/bin/env python3
"""Replace a legacy MySQL account entropy trigger during startup."""

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
    mysql_scalar(url, "DROP TRIGGER accounts_after_update")
    mysql_scalar(url, "CREATE TRIGGER accounts_after_update AFTER UPDATE ON accounts "
                "FOR EACH ROW SET @legacy_entropy_touch=1")
    mysql_scalar(url, "DELETE FROM app_secrets WHERE name='accounts_entropy_bulk_touch_v2'")

    subprocess.run([BINARY, "init-db"], cwd=ROOT, env=env, check=True)
    definition = mysql_scalar(url,
        "SELECT ACTION_STATEMENT FROM information_schema.triggers "
        "WHERE trigger_schema=DATABASE() AND trigger_name='accounts_after_update'")
    assert "bulk_touch_epoch" in definition, definition
    assert mysql_scalar(url,
        "SELECT COUNT(*) FROM app_secrets WHERE name='accounts_entropy_bulk_touch_v2'") == "1"
    print("MySQL legacy account entropy trigger upgraded to row-epoch bulk touch")


if __name__ == "__main__":
    main()
