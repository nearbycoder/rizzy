#!/usr/bin/env python3
"""Compare request tracing headers and storage ledger request IDs with Fizzy."""

import json
import os
import re
import sqlite3
import time

from card_image_parity import call
from storage_ledger_parity import FILE, upload

UUID = re.compile(r"^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$")


def request(target, path, method="GET", payload=None, *, raw_id=None, expected=200):
    headers = {"X-Request-Id": raw_id} if raw_id is not None else None
    status, response_headers, raw = call(
        target, path, method, payload, headers=headers)
    assert status == expected, (target["name"], method, path, status, raw[:200])
    request_id = response_headers.get("X-Request-Id")
    assert request_id is not None, (target["name"], method, path)
    return (json.loads(raw) if raw and response_headers.get_content_type() == "application/json"
            else None), request_id


def entries(target):
    with sqlite3.connect(target["db"]) as db:
        account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?",
                                (int(target["account"]),)).fetchone()[0]
        return db.execute(
            "SELECT operation,delta,request_id FROM storage_entries "
            "WHERE account_id=? ORDER BY rowid", (account_id,)).fetchall()


def check(target):
    _, sanitized = request(target, "/up", raw_id="; X-Hacked-Header: Stuff")
    assert sanitized == "X-Hacked-HeaderStuff", (target["name"], sanitized)
    _, truncated = request(target, "/up", raw_id="X" * 300)
    assert truncated == "X" * 255, (target["name"], truncated)
    _, generated = request(target, "/up")
    assert UUID.fullmatch(generated), (target["name"], generated)
    _, missing = request(target, "/unmapped-request-id-probe", raw_id="missing-route",
                         expected=404)
    assert missing == "missing-route", (target["name"], missing)

    prefix = f"/{target['account']}"
    stamp = time.time_ns()
    first, _ = request(target, prefix + "/boards", "POST",
                       {"board": {"name": f"Trace first {stamp}"}}, expected=201)
    second, _ = request(target, prefix + "/boards", "POST",
                        {"board": {"name": f"Trace second {stamp}"}}, expected=201)
    deleted_boards = set()
    try:
        card, _ = request(target, prefix + f"/boards/{first['id']}/cards", "POST",
                          {"card": {"title": "Trace storage"}}, expected=201)
        blob = upload(target, FILE, "trace.txt", "text/plain")
        markup = (f'<p><action-text-attachment sgid="{blob["attachable_sgid"]}">'
                  '</action-text-attachment></p>')
        _, attached = request(target, prefix + f"/cards/{card['number']}.json", "PATCH",
                              {"card": {"description": markup}}, raw_id="trace-@_ /?!42")
        assert attached == "trace-@_42", (target["name"], attached)
        _, moved = request(target, prefix + f"/cards/{card['number']}/board", "PATCH",
                           {"board_id": second["id"]}, raw_id="move-42")
        _, deleted = request(target, prefix + f"/cards/{card['number']}", "DELETE",
                             raw_id="delete-42", expected=204)
        assert (moved, deleted) == ("move-42", "delete-42")
        trace = entries(target)
        assert trace == [
            ("attach", len(FILE), attached),
            ("transfer_out", -len(FILE), moved),
            ("transfer_in", len(FILE), moved),
            ("detach", -len(FILE), deleted),
        ], (target["name"], trace)
        board_file = FILE + b"board"
        board_blob = upload(target, board_file, "board.txt", "text/plain")
        board_markup = (f'<p><action-text-attachment sgid="{board_blob["attachable_sgid"]}">'
                        '</action-text-attachment></p>')
        _, generated_attach = request(target, prefix + f"/boards/{first['id']}", "PATCH",
                                      {"board": {"public_description": board_markup}})
        assert UUID.fullmatch(generated_attach), (target["name"], generated_attach)
        assert entries(target)[-1] == ("attach", len(board_file), generated_attach)
        replacement_file = FILE + b"replacement"
        replacement = upload(target, replacement_file, "replacement.txt", "text/plain")
        replacement_markup = (
            f'<p><action-text-attachment sgid="{replacement["attachable_sgid"]}">'
            '</action-text-attachment></p>')
        _, long_id = request(target, prefix + f"/boards/{first['id']}", "PATCH",
                             {"board": {"public_description": replacement_markup}},
                             raw_id="X" * 300)
        assert long_id == "X" * 255, (target["name"], long_id)
        assert entries(target)[-2:] == [
            ("detach", -len(board_file), long_id),
            ("attach", len(replacement_file), long_id),
        ], (target["name"], entries(target)[-2:])
        _, board_deleted = request(target, prefix + f"/boards/{first['id']}", "DELETE",
                                   raw_id="board-delete-42", expected=204)
        deleted_boards.add(first["id"])
        assert board_deleted == "board-delete-42", (target["name"], board_deleted)
        assert entries(target)[-1] == ("detach", -len(replacement_file), board_deleted)
        return trace
    finally:
        for board in (first, second):
            if board["id"] not in deleted_boards:
                request(target, prefix + f"/boards/{board['id']}", "DELETE", expected=204)


def main():
    targets = [{"name": name.lower(), "url": os.environ[f"{name}_URL"],
                "account": os.environ[f"{name}_ACCOUNT"],
                "token": os.environ[f"{name}_TOKEN"],
                "db": os.environ[f"{name}_DB_PATH"]}
               for name in ("FIZZY", "RIZZY")]
    source, rust = (check(target) for target in targets)
    assert source == rust, (source, rust)
    print("Request ID parity passed: sanitization, generation, long IDs, and storage entries")


if __name__ == "__main__":
    main()
