#!/usr/bin/env python3
"""Compare owner cancellation and retained records on fresh paired MySQL databases."""

import base64
import hashlib
import json
import os
import re
import select
import subprocess
import time
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path

from card_image_parity import IMAGE, call
from draft_workflow_parity import NoRedirect, request
from sustained_mixed import ROOT, RUST, SOURCE
from sustained_mixed_mysql import mysql_query


def upload_card_image(target, card_number):
    account = target["account"]
    blob = {"filename": "moon.jpg", "byte_size": len(IMAGE),
            "checksum": base64.b64encode(hashlib.md5(IMAGE).digest()).decode(),
            "content_type": "image/jpeg"}
    status, _, raw = call(target, f"/{account}/rails/active_storage/direct_uploads",
                          "POST", {"blob": blob})
    assert status == 200, (target["name"], "direct upload", status, raw[:200])
    upload = json.loads(raw)
    key = upload["key"]
    path = Path(target["storage_dir"])
    if target["name"] == "fizzy":
        path = path / key[:2] / key[2:4]
    path /= key
    target.setdefault("uploaded_files", []).append(path)
    status, _, raw = call(target, upload["direct_upload"]["url"], "PUT", IMAGE,
                          bearer=False, headers=upload["direct_upload"]["headers"])
    assert status == 204, (target["name"], "upload bytes", status, raw[:200])
    status, _, raw = call(target, f"/{account}/cards/{card_number}.json", "PATCH",
                          {"card": {"image": upload["signed_id"]}})
    assert status == 200 and json.loads(raw)["image_url"], (
        target["name"], "attach image", status, raw[:200])
    assert path.read_bytes() == IMAGE, (target["name"], "stored bytes", path)
    return key, path


def run(target, *, image=False):
    account = target["account"]
    status, _, raw = request(target, f"/{account}/users", bearer=True)
    assert status == 200, (target["name"], "users", status)
    owner_id = next(user["id"] for user in json.loads(raw) if user["role"] == "owner")

    status, _, raw = request(target, f"/{account}/boards", "POST",
                             {"board": {"name": "Cancellation parity"}}, bearer=True)
    assert status == 201, (target["name"], "board", status)
    board_id = json.loads(raw)["id"]
    status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                             {"card": {"title": "Cancellation card"}}, bearer=True)
    assert status == 201, (target["name"], "card", status)
    if image:
        target["cancelled_image_key"], target["cancelled_image_path"] = upload_card_image(
            target, json.loads(raw)["number"])

    status, _, page = request(target, f"/{account}/account/settings", accept="text/html")
    assert status == 200 and b"Delete my account" in page, (target["name"], "settings", status)
    match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', page)
    csrf = match.group(1).decode() if match else None
    status, headers, _ = request(target, f"/{account}/account/cancellation", "POST", b"",
                                 accept="text/html", csrf=csrf)
    redirect = urllib.parse.urlparse(headers.get("Location", "")).path
    assert status == 302 and redirect == "/session/menu", (target["name"], "cancel", status, redirect)

    database = target["mysql_database"]
    def scalar(statement):
        return mysql_query(target["mysql_client"], target["mysql_socket"], database, statement)

    initiated_by = scalar(
        "SELECT HEX(initiated_by_id) FROM account_cancellations LIMIT 1"
        if target["name"] == "fizzy" else
        "SELECT initiated_by_id FROM account_cancellations LIMIT 1")
    expected_owner = (format(int(owner_id, 36), "032X")
                      if target["name"] == "fizzy" else owner_id)
    account_count = int(scalar("SELECT COUNT(*) FROM accounts"))
    board_count = int(scalar("SELECT COUNT(*) FROM boards WHERE name='Cancellation parity'"))
    card_count = int(scalar("SELECT COUNT(*) FROM cards WHERE title='Cancellation card'"))
    assert initiated_by == expected_owner and (account_count, board_count, card_count) == (1, 1, 1), (
        target["name"], "retention", initiated_by, expected_owner, account_count, board_count, card_count)

    status, _, raw = request(target, "/my/identity", bearer=True)
    assert status == 200, (target["name"], "identity", status)
    identity_accounts = len(json.loads(raw)["accounts"])
    menu_status, _, menu = request(target, "/session/menu", accept="text/html")
    assert menu_status == 200, (target["name"], "menu", menu_status)
    menu_has_account = f'/{account}/landing'.encode() in menu
    status, _, _ = request(target, f"/{account}/account/settings.json", bearer=True)
    settings_status = status
    assert identity_accounts == 0 and not menu_has_account and settings_status != 200, (
        target["name"], "removed access", identity_accounts, menu_has_account, settings_status)
    return {"redirect": redirect, "cancellation_owner": initiated_by == expected_owner,
            "retained": (account_count, board_count, card_count),
            "identity_accounts": identity_accounts,
            "menu": (menu_status, menu_has_account), "settings_after": settings_status}


def seed_survivor(target, app_env, *, image=False):
    email = "survivor@example.test"
    if target["name"] == "fizzy":
        script = '''require "json"
          identity = Identity.create!(email_address: "survivor@example.test")
          account = Account.create_with_owner(
            account: { external_account_id: 2, name: "Survivor" },
            owner: { name: "Survivor", identity: identity })
          token = identity.access_tokens.create!(description: "Retention", permission: "write")
          puts JSON.generate({ account: account.external_account_id, token: token.token })'''
        output = subprocess.run(["bin/rails", "runner", script], cwd=SOURCE,
                                env=app_env, capture_output=True, text=True, check=True).stdout
        credentials = json.loads(output)
    else:
        output = subprocess.run([RUST, "seed"], cwd=ROOT,
                                env={**app_env, "RIZZY_SEED_EMAIL": email},
                                capture_output=True, text=True, check=True).stdout
        credentials = {"account": int(re.search(r"Account: (\d+)", output).group(1)),
                       "token": re.search(r"Bearer token: (\w+)", output).group(1)}
    assert int(credentials["account"]) == 2, (target["name"], credentials["account"])
    survivor = {**target, "account": "2", "token": credentials["token"],
                "opener": urllib.request.build_opener(NoRedirect())}
    status, _, raw = request(survivor, "/2/boards", "POST",
                             {"board": {"name": "Surviving board"}}, bearer=True)
    assert status == 201, (target["name"], "survivor board", status, raw[:200])
    board_id = json.loads(raw)["id"]
    status, _, raw = request(survivor, f"/2/boards/{board_id}/cards", "POST",
                             {"card": {"title": "Surviving card"}}, bearer=True)
    assert status == 201, (target["name"], "survivor card", status, raw[:200])
    if image:
        target["survivor_image_key"], target["survivor_image_path"] = upload_card_image(
            survivor, json.loads(raw)["number"])
    return credentials["token"]


def age_cancellation(target, days):
    if target["name"] == "fizzy":
        statement = f"UPDATE account_cancellations SET created_at=UTC_TIMESTAMP(6)-INTERVAL {days} DAY"
    else:
        date = (datetime.now(timezone.utc) - timedelta(days=days)).isoformat(
            timespec="milliseconds").replace("+00:00", "Z")
        statement = f"UPDATE account_cancellations SET created_at='{date}'"
    mysql_query(target["mysql_client"], target["mysql_socket"],
                target["mysql_database"], statement)


def source_incineration(source_env):
    subprocess.run(["bin/rails", "runner", "Account::IncinerateDueJob.perform_now"],
                   cwd=SOURCE, env=source_env, capture_output=True, text=True, check=True)


def concurrent_rust_incineration(target, rust_env):
    client = target["mysql_client"]
    database = target["mysql_database"]
    holder = subprocess.Popen(
        [client, "--no-defaults", "--batch", "--skip-column-names", "--unbuffered",
         f"--socket={target['mysql_socket']}", "-uroot", f"--database={database}"],
        env={**os.environ,
             "LD_LIBRARY_PATH": str(Path(client).resolve().parents[1] / "lib")},
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, bufsize=1)
    workers = []
    try:
        holder.stdin.write("START TRANSACTION; SELECT HEX(account_id) FROM account_cancellations FOR UPDATE;\n")
        holder.stdin.flush()
        ready, _, _ = select.select([holder.stdout], [], [], 5)
        assert ready and holder.stdout.readline().strip(), "Could not lock the cancellation row"
        for _ in range(2):
            workers.append(subprocess.Popen([RUST, "incinerate-due"], cwd=ROOT,
                                            env=rust_env, stdout=subprocess.PIPE,
                                            stderr=subprocess.PIPE, text=True))
        waits_sql = ("SELECT COUNT(DISTINCT w.REQUESTING_THREAD_ID) "
                     "FROM performance_schema.data_lock_waits w "
                     "JOIN performance_schema.data_locks l "
                     "ON l.ENGINE_LOCK_ID=w.REQUESTING_ENGINE_LOCK_ID "
                     f"WHERE l.OBJECT_SCHEMA='{database}' "
                     "AND l.OBJECT_NAME='account_cancellations'")
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            waiting = int(mysql_query(client, target["mysql_socket"], database, waits_sql))
            if waiting >= 2:
                break
            assert all(worker.poll() is None for worker in workers), (
                "incineration worker exited before row lock", [worker.poll() for worker in workers])
            time.sleep(.05)
        else:
            raise AssertionError("Both incineration workers did not reach the same row lock")
        holder.stdin.write("COMMIT;\n")
        holder.stdin.flush()
        holder.stdin.close()
        assert holder.wait(timeout=10) == 0, holder.stderr.read()[-2000:]
        for worker in workers:
            stdout, stderr = worker.communicate(timeout=60)
            assert worker.returncode == 0, (worker.returncode, stdout[-2000:], stderr[-2000:])
    finally:
        if holder.poll() is None:
            holder.kill()
        holder.wait()
        for worker in workers:
            if worker.poll() is None:
                worker.kill()
            worker.wait()


def source_purge_enqueued_blob(source_env, key):
    script = '''blob = ActiveStorage::Blob.find_by!(key: ENV.fetch("RIZZY_TEST_BLOB_KEY"))
      queued = SolidQueue::Job.where(class_name: "ActiveStorage::PurgeJob", finished_at: nil).select do |record|
        payload = record.arguments
        payload.fetch("arguments").first.fetch("_aj_globalid") == blob.to_global_id.to_s
      end
      raise "Expected one queued purge for cancelled card image, got #{queued.length}" unless queued.one?
      ActiveJob::Base.deserialize(queued.first.arguments).perform_now
      raise "Blob remained after purge" if ActiveStorage::Blob.exists?(blob.id)
      puts "purged"
    '''
    completed = subprocess.run(["bin/rails", "runner", script], cwd=SOURCE,
                               env={**source_env, "RIZZY_TEST_BLOB_KEY": key},
                               capture_output=True, text=True)
    assert completed.returncode == 0 and completed.stdout.strip().endswith("purged"), (
        completed.stdout[-2000:], completed.stderr[:2000])


def retained_counts(target):
    statement = ("SELECT (SELECT COUNT(*) FROM accounts WHERE external_account_id=1),"
                 "(SELECT COUNT(*) FROM accounts WHERE external_account_id=2),"
                 "(SELECT COUNT(*) FROM boards WHERE name='Cancellation parity'),"
                 "(SELECT COUNT(*) FROM cards WHERE title='Cancellation card'),"
                 "(SELECT COUNT(*) FROM boards WHERE name='Surviving board'),"
                 "(SELECT COUNT(*) FROM cards WHERE title='Surviving card')")
    raw = mysql_query(target["mysql_client"], target["mysql_socket"],
                      target["mysql_database"], statement)
    return tuple(int(value) for value in raw.split("\t"))


def wait_for_deletion(target):
    for _ in range(100):
        counts = retained_counts(target)
        if counts[0] == 0:
            return counts
        time.sleep(.05)
    raise AssertionError((target["name"], "incineration timeout", counts))


def survivor_access(target, token):
    survivor = {**target, "token": token,
                "opener": urllib.request.build_opener(NoRedirect())}
    status, _, _ = request(survivor, "/2/account/settings.json", bearer=True)
    assert status == 200, (target["name"], "survivor access", status)
    return status
