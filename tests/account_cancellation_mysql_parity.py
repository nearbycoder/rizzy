#!/usr/bin/env python3
"""Compare owner cancellation and retained records on fresh paired MySQL databases."""

import json
import re
import subprocess
import time
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone

from draft_workflow_parity import NoRedirect, request
from sustained_mixed import ROOT, RUST, SOURCE
from sustained_mixed_mysql import mysql_query


def run(target):
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


def seed_survivor(target, app_env):
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
    survivor = {**target, "token": credentials["token"],
                "opener": urllib.request.build_opener(NoRedirect())}
    status, _, raw = request(survivor, "/2/boards", "POST",
                             {"board": {"name": "Surviving board"}}, bearer=True)
    assert status == 201, (target["name"], "survivor board", status, raw[:200])
    board_id = json.loads(raw)["id"]
    status, _, raw = request(survivor, f"/2/boards/{board_id}/cards", "POST",
                             {"card": {"title": "Surviving card"}}, bearer=True)
    assert status == 201, (target["name"], "survivor card", status, raw[:200])
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
