#!/usr/bin/env python3
"""Verify mobile configuration and user-scoped timezone persistence."""
import hashlib
import json
import os
import re
import sqlite3
import subprocess
import tempfile
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from join import call


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "settings.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        seed = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seed.stdout).group(1)
        owner_token = re.search(r"Bearer token: (\w+)", seed.stdout).group(1)
        port, server = start(env)
        try:
            for platform in ("android", "ios"):
                path = f"/client_configurations/{platform}_v1.json"
                expected = json.loads((ROOT / "upstream" / "app" / "views" / "client_configurations" / f"{platform}_v1.json").read_text())
                for prefix in ("", f"/{account}"):
                    actual, headers = call(port, prefix + path)
                    assert actual == expected
                    assert headers["Cache-Control"] == "max-age=60, public"
                    assert headers["Content-Type"] == "application/json; charset=utf-8"
                    without_suffix, _ = call(port, prefix + path.removesuffix(".json"))
                    assert without_suffix == expected
                from_query, _ = call(port, "/client_configurations/" + platform + "_v1?format=json", accept="text/html")
                assert from_query == expected
            for path in ("android_v2000.json", "ios_v2000.json", "android_v1.html"):
                call(port, "/client_configurations/" + path, status=500)
            for path in ("blackberry_v1.json", "android_va.json"):
                call(port, "/client_configurations/" + path, status=404)

            path = f"/{account}/my/timezone"
            call(port, path, "PATCH", {"timezone_name": "America/Chicago"}, status=302)
            with sqlite3.connect(database) as db:
                account_id, owner_id = db.execute("SELECT account_id,id FROM users WHERE role='owner'").fetchone()
                db.execute("INSERT INTO identities(id,email_address,created_at) VALUES('tz-identity','tz@example.com','2026-01-01T00:00:00Z')")
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES('tz-user',?,'tz-identity','TZ','member','2026-01-01T00:00:00Z')", (account_id,))
                db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES('tz-token','tz-identity',?,'Test','write','2026-01-01T00:00:00Z')", (hashlib.sha256(b'tz-token').hexdigest(),))
            call(port, path, "PATCH", {"timezone_name": "America/Chicago"}, bearer=owner_token, status=204)
            call(port, path, "PUT", {"timezone_name": "America/New_York"}, bearer="tz-token", status=204)
            with sqlite3.connect(database) as db:
                rows = db.execute("SELECT user_id,account_id,timezone_name FROM user_timezones ORDER BY user_id").fetchall()
                assert rows == [(owner_id, account_id, "America/Chicago"), ("tz-user", account_id, "America/New_York")]
            call(port, path, "PATCH", {"timezone_name": None}, bearer="tz-token", status=204)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT timezone_name FROM user_timezones WHERE user_id='tz-user'").fetchone()[0] is None
        finally:
            stop(server)
    print("client configuration and timezone checks passed")


if __name__ == "__main__":
    main()
