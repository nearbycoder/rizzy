#!/usr/bin/env python3
"""Compare owner cancellation and retained records on fresh paired MySQL databases."""

import json
import re
import urllib.parse

from draft_workflow_parity import request
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
