#!/usr/bin/env python3
"""Compare the 99/100-assignee boundary on disposable Fizzy and Rizzy cards."""

import http.cookiejar
import json
import os
import re
import sqlite3
import sys
import tempfile
import urllib.request
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from activity_event_board_parity import database_id  # noqa: E402
from activity_particulars_parity import public_id, stored_id  # noqa: E402
from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def api(target, path, method="GET", payload=None, expected=200):
    status, headers, body = request(target, f"/{target['account']}{path}", method,
                                    payload, bearer=True)
    assert status == expected, (target["name"], method, path, status, body[:200])
    return json.loads(body) if body else None


def clone(table, row, **changes):
    values = {**dict(row), **changes}
    return f"INSERT INTO {table} ({','.join(values)}) VALUES ({','.join('?' for _ in values)})", tuple(values.values())


def add_people_and_assignments(target, board_id, card_id):
    with sqlite3.connect(target["db"]) as db:
        db.row_factory = sqlite3.Row
        db.execute("PRAGMA foreign_keys=ON")
        board_key = database_id(target, board_id)
        card_key = database_id(target, card_id)
        owner = db.execute("SELECT * FROM users WHERE account_id=(SELECT id FROM accounts WHERE external_account_id=?) AND role='owner'", (target["account"],)).fetchone()
        assert owner, target["name"]
        identity = db.execute("SELECT * FROM identities WHERE id=?", (owner["identity_id"],)).fetchone()
        access = db.execute("SELECT * FROM accesses WHERE board_id=? AND user_id=?", (board_key, owner["id"])).fetchone()
        assignment = db.execute("SELECT * FROM assignments WHERE card_id=? AND assignee_id=?", (card_key, owner["id"])).fetchone()
        assert identity and access and assignment, target["name"]
        people = []
        for index in range(100):
            identity_int, user_int, access_int, assignment_int = (uuid.uuid4().int for _ in range(4))
            identity_id, user_id, access_id, assignment_id = (
                stored_id(target, value) for value in
                (identity_int, user_int, access_int, assignment_int))
            db.execute(*clone("identities", identity, id=identity_id,
                              email_address=f"assignment-{index:03d}-{uuid.uuid4().hex}@example.test"))
            db.execute(*clone("users", owner, id=user_id, identity_id=identity_id,
                              name=f"Member {index:03d}", role="member"))
            access_changes = {"user_id": user_id, "involvement": "access_only"}
            if "id" in access.keys():
                access_changes["id"] = access_id
            db.execute(*clone("accesses", access, **access_changes))
            people.append(public_id(target, user_int))
            if index < 98:
                db.execute(*clone("assignments", assignment, id=assignment_id,
                                  assignee_id=user_id))
        count = db.execute("SELECT COUNT(*) FROM assignments WHERE card_id=?", (card_key,)).fetchone()[0]
        assert count == 99, (target["name"], count)
    return people[-2], people[-1]


def picker(target, number, at_limit, expected_unassigned=None):
    status, _, body = request(target, f"/{target['account']}/cards/{number}/assignments/new",
                              accept="text/html")
    assert status == 200, (target["name"], status, body[:160])
    frame = body.decode()
    assert f'data-assignment-limit-count-value="{100 if at_limit else 99}"' in frame
    warning = re.search(r'<div class="popup__footer"([^>]*)>\s*Maximum 100 assignees', frame)
    assert warning and " hidden" in warning.group(1), (target["name"], frame[frame.find("Maximum")-180:frame.find("Maximum")+60])
    unassigned = re.findall(r'<li class="popup__item"([^>]*)>', frame)
    unassigned = [item for item in unassigned if 'data-assignment-limit-target="unassigned"' in item]
    expected_unassigned = expected_unassigned if expected_unassigned is not None else (1 if at_limit else 2)
    assert len(unassigned) == expected_unassigned, (target["name"], at_limit, len(unassigned))
    assert all(" hidden" not in item for item in unassigned), (target["name"], at_limit, unassigned)
    return frame


def check(target):
    login(target)
    board = api(target, "/boards", "POST", {"board": {"name": "Assignment limit", "all_access": False}}, 201)
    board_id = board["id"]
    try:
        card = api(target, f"/boards/{board_id}/cards", "POST",
                   {"card": {"title": "Assignment limit"}}, 201)
        number, card_id = card["number"], card["id"]
        api(target, f"/cards/{number}/publish", "POST", {}, 201)
        owner = next(user["id"] for user in api(target, "/users") if user["role"] == "owner")
        api(target, f"/cards/{number}/assignments", "POST", {"assignee_id": owner}, 204)
        candidate, remaining = add_people_and_assignments(target, board_id, card_id)
        before = picker(target, number, False)
        assert f'assignee_id={candidate}' in before and f'assignee_id={remaining}' in before
        api(target, f"/cards/{number}/assignments", "POST", {"assignee_id": candidate}, 204)
        full = picker(target, number, True)
        assert f'assignee_id={remaining}' in full
        status, headers, body = request(target, f"/{target['account']}/cards/{number}/assignments",
                                        "POST", {"assignee_id": remaining}, bearer=True)
        assert status == 422, (target["name"], "at-limit JSON", status, body[:200])
        after = picker(target, number, True)
        assert f'assignee_id={remaining}' in after
        with sqlite3.connect(target["db"]) as db:
            count = db.execute("SELECT COUNT(*) FROM assignments WHERE card_id=?",
                               (database_id(target, card_id),)).fetchone()[0]
        assert count == 100, (target["name"], count)
        api(target, f"/cards/{number}/assignments", "POST", {"assignee_id": owner}, 204)
        api(target, f"/cards/{number}/assignments", "POST", {"assignee_id": remaining}, 204)
        status_self, self_headers, self_body = request(
            target, f"/{target['account']}/cards/{number}/self_assignment",
            "POST", {}, bearer=True)
        assert status_self == 422, (target["name"], "at-limit self assignment", status_self,
                                    self_body[:200])
        api(target, f"/cards/{number}/assignments", "POST", {"assignee_id": candidate}, 204)
        picker(target, number, False, expected_unassigned=1)
        return {"before": 99, "full": 100, "rejected_status": status,
                "rejected_type": headers.get_content_type(), "rejected_body": body,
                "self_rejected_status": status_self,
                "self_rejected_type": self_headers.get_content_type(),
                "self_rejected_body": self_body,
                "remaining_after_reject": count, "after_removal": 99}
    finally:
        api(target, f"/boards/{board_id}", "DELETE", expected=204)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-assignment-limit-") as temporary:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(directory, 2)
                else:
                    env = rust_env(directory)
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    env, credentials = prepare_rust(directory, env)
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url, "account": str(credentials["account"]),
                          "token": credentials["token"], "email": "owner@example.com",
                          "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                          "opener": urllib.request.build_opener(
                              urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                              NoRedirect())}
                results[name] = check(target)
            assert results["fizzy"] == results["rizzy"], results
            print("PASS paired 99/100-assignee picker and rejection boundary")
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
