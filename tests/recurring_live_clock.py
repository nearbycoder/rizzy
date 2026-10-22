#!/usr/bin/env python3
"""Observe real recurring cleanup clock boundaries on disposable SQLite apps."""

import os
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"
CLAIMS = {
    "auth": ("magic_link_cleanup_slot", "transfer_cleanup_slot"),
    "exports": ("export_cleanup_slot",),
    "imports": ("import_cleanup_slot",),
}


def stamp(offset):
    return (datetime.now(timezone.utc) + offset).isoformat(
        timespec="milliseconds").replace("+00:00", "Z")


def timezone_for(target, kind):
    utc_minutes = target.hour * 60 + target.minute
    if kind == "auth":
        offset = min((((hour * 60 - utc_minutes + 720) % 1440) - 720
                      for hour in range(0, 24, 4)), key=abs)
    else:
        local_minute = 20 if kind == "exports" else 25
        offset = local_minute - target.minute
    sign = "-" if offset >= 0 else "+"  # POSIX TZ signs are west of UTC.
    absolute = abs(offset)
    return f"RIZ{sign}{absolute // 60}:{absolute % 60:02d}"


def port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


def setup(kind, directory):
    database = directory / "rizzy.sqlite3"
    exports = directory / "exports"
    imports = directory / "imports"
    exports.mkdir()
    imports.mkdir()
    env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}",
           "RIZZY_EXPORT_DIR": str(exports), "RIZZY_IMPORT_DIR": str(imports),
           "RIZZY_STORAGE_DIR": str(directory / "storage"),
           "RIZZY_SEED_EMAIL": f"live-{kind}@example.test",
           "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
           "RIZZY_DISABLE_WEBHOOK_CLEANUP": "1",
           "RIZZY_DISABLE_AUTO_POSTPONE": "1", "RIZZY_DISABLE_TAG_CLEANUP": "1",
           "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
    subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                   capture_output=True, text=True, check=True)
    with sqlite3.connect(database) as db:
        identity = db.execute("SELECT id FROM identities WHERE email_address=?",
                              (env["RIZZY_SEED_EMAIL"],)).fetchone()[0]
        account, owner = db.execute("SELECT a.id,u.id FROM accounts a JOIN users u "
                                    "ON u.account_id=a.id WHERE u.identity_id=? AND u.role='owner'",
                                    (identity,)).fetchone()
        old = stamp(-timedelta(hours=25))
        recent = stamp(-timedelta(hours=23))
        if kind == "auth":
            db.execute("INSERT INTO magic_links(id,identity_id,code,purpose,expires_at,created_at) "
                       "VALUES('magic-old',?,'oldcode','sign_in',?,?)", (identity, old, old))
            db.execute("INSERT INTO magic_links(id,identity_id,code,purpose,expires_at,created_at) "
                       "VALUES('magic-new',?,'newcode','sign_in',?,?)",
                       (identity, stamp(timedelta(hours=1)), recent))
            db.execute("INSERT INTO identity_transfers(id,identity_id,token,created_at) "
                       "VALUES('transfer-old',?,'token-old',?)", (identity, old))
            db.execute("INSERT INTO identity_transfers(id,identity_id,token,created_at) "
                       "VALUES('transfer-new',?,'token-new',?)",
                       (identity, stamp(-timedelta(hours=3))))
        elif kind == "exports":
            for name, completed in (("old", old), ("new", recent)):
                db.execute("INSERT INTO exports(id,account_id,user_id,kind,status,created_at,updated_at,completed_at) "
                           "VALUES(?,?,?,'user','completed',?,?,?)",
                           (f"export-{name}", account, owner, completed, completed, completed))
                (exports / f"export-{name}.zip").write_bytes(b"archive")
        else:
            for name, completed in (("old", old), ("new", recent)):
                db.execute("INSERT INTO imports(id,account_id,identity_id,status,created_at,updated_at,completed_at) "
                           "VALUES(?,?,?,'completed',?,?,?)",
                           (f"import-{name}", account, identity, completed, completed, completed))
                (imports / f"import-{name}.zip").write_bytes(b"archive")
    return database, env, exports, imports


def check(kind, database, exports, imports, target_slot, *, finished):
    with sqlite3.connect(database) as db:
        if kind == "auth":
            assert db.execute("SELECT id FROM magic_links ORDER BY id").fetchall() == (
                [("magic-new",)] if finished else [("magic-new",), ("magic-old",)])
            assert db.execute("SELECT id FROM identity_transfers ORDER BY id").fetchall() == (
                [("transfer-new",)] if finished else [("transfer-new",), ("transfer-old",)])
        else:
            table = kind
            prefix = "export" if kind == "exports" else "import"
            assert db.execute(f"SELECT id FROM {table} ORDER BY id").fetchall() == (
                [(f"{prefix}-new",)] if finished else [(f"{prefix}-new",), (f"{prefix}-old",)])
            directory = exports if kind == "exports" else imports
            assert (directory / f"{prefix}-new.zip").exists()
            assert (directory / f"{prefix}-old.zip").exists() != finished
        placeholders = ",".join("?" for _ in CLAIMS[kind])
        claims = db.execute(f"SELECT name,value FROM app_secrets WHERE name IN ({placeholders})",
                            CLAIMS[kind]).fetchall()
    if finished:
        assert sorted(claims) == [(name, target_slot) for name in sorted(CLAIMS[kind])], claims
    else:
        assert claims == [], claims


def main():
    subprocess.run(["cargo", "build", "--locked", "--quiet"], cwd=ROOT, check=True)
    processes = []
    with tempfile.TemporaryDirectory(prefix="rizzy-live-clock-") as temporary:
        root = Path(temporary)
        fixtures = {}
        try:
            for kind in CLAIMS:
                directory = root / kind
                directory.mkdir()
                fixtures[kind] = setup(kind, directory)
            current = datetime.now(timezone.utc)
            target = current.replace(second=0, microsecond=0) + timedelta(minutes=2)
            target_slot = target.strftime("%Y-%m-%dT%H")
            for kind, (database, env, exports, imports) in fixtures.items():
                address = f"127.0.0.1:{port()}"
                env = {**env, "TZ": timezone_for(target, kind), "RIZZY_BIND": address}
                log = root / kind / "server.log"
                output = log.open("w")
                process = subprocess.Popen([BINARY], cwd=ROOT, env=env,
                                           stdout=output, stderr=subprocess.STDOUT)
                output.close()
                processes.append((process, log))
                for _ in range(100):
                    try:
                        with urllib.request.urlopen(f"http://{address}/up", timeout=.2) as response:
                            assert response.status == 200
                        break
                    except OSError:
                        time.sleep(.05)
                else:
                    raise AssertionError((kind, "server did not start", log.read_text()[-1000:]))
                check(kind, database, exports, imports, target_slot, finished=False)
            assert datetime.now(timezone.utc) < target, "Fixture setup crossed its target minute"
            while datetime.now(timezone.utc) < target:
                time.sleep(.2)
            deadline = time.monotonic() + 70
            while True:
                try:
                    for kind, (database, _, exports, imports) in fixtures.items():
                        check(kind, database, exports, imports, target_slot, finished=True)
                    break
                except AssertionError:
                    if time.monotonic() >= deadline:
                        raise
                    time.sleep(.5)
            for process, log in processes:
                assert process.poll() is None, (process.returncode, log.read_text()[-1000:])
            print("Live clock slots ran magic-link, transfer, export, and import cleanup once")
        finally:
            for process, log in processes:
                process.terminate()
                try:
                    process.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)
                if process.returncode not in (-15, 0):
                    print(log.read_text()[-1000:])


if __name__ == "__main__":
    main()
