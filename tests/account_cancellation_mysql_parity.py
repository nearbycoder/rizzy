#!/usr/bin/env python3
"""Compare owner cancellation and retained records on fresh paired MySQL databases."""

import base64
import email
import html
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
from email import policy
from email.parser import BytesParser
from pathlib import Path

from card_image_parity import IMAGE, call
from embed_parity import FILE
from draft_workflow_parity import NoRedirect, request
from sustained_mixed import ROOT, RUST, SOURCE
from sustained_mixed_mysql import mysql_query


def upload_blob(target, content, filename, content_type):
    account = target["account"]
    blob = {"filename": filename, "byte_size": len(content),
            "checksum": base64.b64encode(hashlib.md5(content).digest()).decode(),
            "content_type": content_type}
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
    status, _, raw = call(target, upload["direct_upload"]["url"], "PUT", content,
                          bearer=False, headers=upload["direct_upload"]["headers"])
    assert status == 204, (target["name"], "upload bytes", status, raw[:200])
    assert path.read_bytes() == content, (target["name"], "stored bytes", path)
    return upload, path


def upload_card_image(target, card_number):
    upload, path = upload_blob(target, IMAGE, "moon.jpg", "image/jpeg")
    account = target["account"]
    status, _, raw = call(target, f"/{account}/cards/{card_number}.json", "PATCH",
                          {"card": {"image": upload["signed_id"]}})
    assert status == 200 and json.loads(raw)["image_url"], (
        target["name"], "attach image", status, raw[:200])
    return upload["key"], path


def upload_avatar(target, user_id):
    upload, path = upload_blob(target, IMAGE, "avatar.jpg", "image/jpeg")
    status, _, raw = call(target, f"/{target['account']}/users/{user_id}.json", "PATCH",
                          {"user": {"avatar": upload["signed_id"]}})
    assert status == 200 and json.loads(raw)["avatar_url"], (
        target["name"], "attach avatar", status, raw[:200])
    return upload["key"], path, IMAGE


def upload_embeds(target, card_number):
    file_upload, file_path = upload_blob(target, FILE, "notes.txt", "text/plain")
    markup = (f'<p>Read <action-text-attachment sgid="{file_upload["attachable_sgid"]}">'
              '</action-text-attachment></p>')
    status, _, raw = call(target, f"/{target['account']}/cards/{card_number}.json", "PATCH",
                          {"card": {"description": markup}})
    assert status == 200 and json.loads(raw)["has_attachments"], (
        target["name"], "attach card file", status, raw[:200])
    image_upload, image_path = upload_blob(target, IMAGE, "comment.jpg", "image/jpeg")
    markup = (f'<p><action-text-attachment sgid="{image_upload["attachable_sgid"]}">'
              '</action-text-attachment></p>')
    status, _, raw = call(target, f"/{target['account']}/cards/{card_number}/comments.json",
                          "POST", {"comment": {"body": markup}})
    assert status == 201 and json.loads(raw)["body"]["html"], (
        target["name"], "attach comment image", status, raw[:200])
    return [(file_upload["key"], file_path, FILE),
            (image_upload["key"], image_path, IMAGE)]


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
        target["cancelled_files"] = [
            (target["cancelled_image_key"], target["cancelled_image_path"], IMAGE),
            upload_avatar(target, owner_id),
            *upload_embeds(target, json.loads(raw)["number"]),
        ]

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
    scoped_status, scoped_headers, _ = request(target, f"/{account}/my/identity.json", bearer=True)
    scoped_redirect = urllib.parse.urlparse(scoped_headers.get("Location", "")).path
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
            "scoped_identity": (scoped_status, scoped_redirect),
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


def source_cancellation_mail(source_env):
    script = '''require "json"
      message = AccountMailer.cancellation(Account::Cancellation.first).message
      puts JSON.generate({to: message.to, subject: message.subject,
                          text: message.text_part.body.decoded,
                          html: message.html_part.body.decoded})'''
    completed = subprocess.run(["bin/rails", "runner", script], cwd=SOURCE,
                               env=source_env, capture_output=True, text=True, check=True)
    return json.loads(completed.stdout.strip().splitlines()[-1])


def normalized_mail(message, source=False):
    text = message["text"]
    markup = message["html"]
    subject = message["subject"]
    if source:
        text = text.replace("Fizzy", "Rizzy").replace("support@fizzy.do", "support@example.test")
        markup = markup.replace("Fizzy", "Rizzy").replace("support@fizzy.do", "support@example.test")
        subject = subject.replace("Fizzy", "Rizzy")
    content = html.unescape(re.sub(r"<[^>]+>", " ", markup))
    return {"subject": subject,
            "text": " ".join(text.split()),
            "html_markers": tuple(phrase in " ".join(content.lower().split()) for phrase in (
                "your rizzy account", "was cancelled", "what happens now?",
                "no one can access", "deleted in about 1 month", "changed your mind?",
                "support@example.test")),
            "support_link": 'mailto:support@example.test' in markup}


def deliver_rust_cancellation_mail(target, rust_env, directory):
    database = target["mysql_database"]
    def status():
        return mysql_query(target["mysql_client"], target["mysql_socket"], database,
                           "SELECT status FROM cancellation_mail_jobs")
    assert status() == "0", (target["name"], "queued mail", status())
    failed = directory / "fail-sendmail.sh"
    failed.write_text("#!/bin/sh\nexit 1\n")
    failed.chmod(0o700)
    capture = directory / "cancellation-mail.eml"
    success = directory / "capture-sendmail.sh"
    success.write_text('#!/bin/sh\ncat > "$RIZZY_MAIL_CAPTURE"\n')
    success.chmod(0o700)
    mail_env = {**rust_env, "RIZZY_MAIL_FROM": "support@example.test",
                "RIZZY_SUPPORT_EMAIL": "support@example.test",
                "RIZZY_MAIL_CAPTURE": str(capture)}
    def deliver(executable, expected):
        completed = subprocess.run([RUST, "deliver-cancellation-mail"], cwd=ROOT,
                                   env={**mail_env, "RIZZY_SENDMAIL": str(executable)},
                                   capture_output=True, text=True, check=True)
        assert status() == expected, (target["name"], "mail retry", expected, completed.stdout)
        return completed.stdout
    assert "Delivered 0" in deliver(failed, "0")
    recent = datetime.now(timezone.utc).isoformat(timespec="milliseconds").replace("+00:00", "Z")
    mysql_query(target["mysql_client"], target["mysql_socket"], database,
                f"UPDATE cancellation_mail_jobs SET status=1,claimed_at='{recent}'")
    assert "Delivered 0" in deliver(success, "1")
    assert not capture.exists(), "A fresh claim was stolen"
    stale = (datetime.now(timezone.utc) - timedelta(minutes=16)).isoformat(
        timespec="milliseconds").replace("+00:00", "Z")
    mysql_query(target["mysql_client"], target["mysql_socket"], database,
                f"UPDATE cancellation_mail_jobs SET claimed_at='{stale}'")
    assert "Delivered 1" in deliver(success, "2")
    parsed = BytesParser(policy=policy.default).parsebytes(capture.read_bytes())
    assert parsed.is_multipart(), parsed.get_content_type()
    parts = {part.get_content_type(): part.get_content() for part in parsed.iter_parts()}
    assert set(parts) == {"text/plain", "text/html"}, parts.keys()
    return {"to": str(parsed["To"]), "subject": str(parsed["Subject"]),
            "text": parts["text/plain"], "html": parts["text/html"]}


def concurrent_rust_mail_delivery(target, rust_env, directory):
    client = target["mysql_client"]
    database = target["mysql_database"]
    capture_dir = directory / "concurrent-mail"
    capture_dir.mkdir()
    sendmail = directory / "concurrent-sendmail.sh"
    sendmail.write_text('#!/bin/sh\nfile=$(mktemp "$RIZZY_MAIL_CAPTURE_DIR/message.XXXXXX") || exit 1\ncat > "$file"\n')
    sendmail.chmod(0o700)
    mail_env = {**rust_env, "RIZZY_SENDMAIL": str(sendmail),
                "RIZZY_MAIL_FROM": "support@example.test",
                "RIZZY_SUPPORT_EMAIL": "support@example.test",
                "RIZZY_MAIL_CAPTURE_DIR": str(capture_dir)}
    holder = subprocess.Popen(
        [client, "--no-defaults", "--batch", "--skip-column-names", "--unbuffered",
         f"--socket={target['mysql_socket']}", "-uroot", f"--database={database}"],
        env={**os.environ,
             "LD_LIBRARY_PATH": str(Path(client).resolve().parents[1] / "lib")},
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
        text=True, bufsize=1)
    workers = []
    try:
        holder.stdin.write("START TRANSACTION; SELECT id FROM cancellation_mail_jobs FOR UPDATE;\n")
        holder.stdin.flush()
        ready, _, _ = select.select([holder.stdout], [], [], 5)
        assert ready and holder.stdout.readline().strip(), "Could not lock cancellation mail job"
        for _ in range(2):
            workers.append(subprocess.Popen([RUST, "deliver-cancellation-mail"], cwd=ROOT,
                                            env=mail_env, stdout=subprocess.PIPE,
                                            stderr=subprocess.PIPE, text=True))
        waits_sql = ("SELECT COUNT(DISTINCT w.REQUESTING_THREAD_ID) "
                     "FROM performance_schema.data_lock_waits w "
                     "JOIN performance_schema.data_locks l "
                     "ON l.ENGINE_LOCK_ID=w.REQUESTING_ENGINE_LOCK_ID "
                     f"WHERE l.OBJECT_SCHEMA='{database}' "
                     "AND l.OBJECT_NAME='cancellation_mail_jobs'")
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            waiting = int(mysql_query(client, target["mysql_socket"], database, waits_sql))
            if waiting >= 2:
                break
            assert all(worker.poll() is None for worker in workers), (
                "mail worker exited before row lock", [worker.poll() for worker in workers])
            time.sleep(.05)
        else:
            raise AssertionError("Both mail workers did not reach the same row lock")
        holder.stdin.write("COMMIT;\n")
        holder.stdin.flush()
        holder.stdin.close()
        assert holder.wait(timeout=10) == 0, holder.stderr.read()[-2000:]
        counts = []
        for worker in workers:
            stdout, stderr = worker.communicate(timeout=60)
            assert worker.returncode == 0, (worker.returncode, stdout[-2000:], stderr[-2000:])
            counts.append(int(re.search(r"Delivered (\d+) account cancellation emails", stdout).group(1)))
        assert sorted(counts) == [0, 1], counts
    finally:
        if holder.poll() is None:
            holder.kill()
        holder.wait()
        for worker in workers:
            if worker.poll() is None:
                worker.kill()
            worker.wait()
    status = mysql_query(client, target["mysql_socket"], database,
                         "SELECT status FROM cancellation_mail_jobs")
    assert status == "2", status
    files = list(capture_dir.glob("message.*"))
    assert len(files) == 1, files
    repeated = subprocess.run([RUST, "deliver-cancellation-mail"], cwd=ROOT,
                              env=mail_env, capture_output=True, text=True, check=True)
    assert "Delivered 0" in repeated.stdout and len(list(capture_dir.glob("message.*"))) == 1
    parsed = BytesParser(policy=policy.default).parsebytes(files[0].read_bytes())
    assert parsed.is_multipart(), parsed.get_content_type()
    parts = {part.get_content_type(): part.get_content() for part in parsed.iter_parts()}
    assert set(parts) == {"text/plain", "text/html"}, parts.keys()
    return {"to": str(parsed["To"]), "subject": str(parsed["Subject"]),
            "text": parts["text/plain"], "html": parts["text/html"]}


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


def stored_blob_count(target, key):
    table = "active_storage_blobs" if target["name"] == "fizzy" else "storage_blobs"
    column = "key" if target["name"] == "fizzy" else "storage_key"
    assert re.fullmatch(r"[a-zA-Z0-9_-]+", key), key
    return int(mysql_query(target["mysql_client"], target["mysql_socket"],
                           target["mysql_database"],
                           f"SELECT COUNT(*) FROM `{table}` WHERE `{column}`='{key}'"))


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
