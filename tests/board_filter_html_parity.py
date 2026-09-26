#!/usr/bin/env python3
"""Compare filtered board HTML and pagination with the pinned Fizzy app."""
import http.cookiejar
import json
import os
import re
import sqlite3
import time
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request

GRID = '<section class="cards cards--grid"'


def run(target):
    try:
        login(target)
    except AssertionError as error:
        raise AssertionError(f"{target['name']} sign-in failed: {error}") from error
    account = target["account"]
    marker = f"boardfilter{time.time_ns()}"
    status, _, raw = request(target, f"/{account}/boards", "POST", {"board": {"name": marker}}, bearer=True)
    assert status == 201, (target["name"], status, raw[:200])
    board_id = json.loads(raw)["id"]
    board_path = f"/{account}/boards/{board_id}"
    try:
        for index in range(17):
            status, _, raw = request(target, board_path + "/cards", "POST", {"card": {"title": f"{marker} card {index:02d}"}}, bearer=True)
            assert status == 201, (status, raw[:200])
            if index == 0:
                tagged_number = json.loads(raw)["number"]
                owner_id = json.loads(raw)["creator"]["id"]
        tag_title = f"tag{marker}"
        status, _, _ = request(target, f"/{account}/cards/{tagged_number}/taggings", "POST",
                               {"tag_title": tag_title}, bearer=True)
        assert status == 204, (target["name"], "tag", status)
        status, _, raw = request(target, f"/{account}/tags", bearer=True)
        assert status == 200
        tag_id = next(tag["id"] for tag in json.loads(raw) if tag["title"] == tag_title)
        special = {}
        for state in ("closed", "not_now"):
            title = f"{state} separate {marker}"
            status, _, raw = request(target, board_path + "/cards", "POST", {"card": {"title": title}}, bearer=True)
            assert status == 201, status
            number = json.loads(raw)["number"]
            status, _, raw = request(target, f"/{account}/cards/{number}/{ 'closure' if state == 'closed' else 'not_now' }", "POST", bearer=True)
            assert status == 204, (state, status, raw[:200])
            special[state] = title

        def html(query=""):
            status, _, body = request(target, board_path + query, accept="text/html")
            assert status == 200, (target["name"], query, status, body[:200])
            return body.decode()

        plain = html()
        assert "card-columns" in plain and "cards--maybe" in plain and GRID not in plain
        if target["name"] == "rizzy":
            assert 'cards--maybe is-expanded' in plain and 'board-tools__watch-toggle' in plain
            assert 'data-card-number=' in plain and 'class="bottom-bar"' in plain
            target["extra_headers"] = {"X-Rizzy-Card-Fragment": "1"}
            try:
                status, _, fragment = request(target, board_path + "/columns/stream?page=2", accept="text/html")
            finally:
                target["extra_headers"].pop("X-Rizzy-Card-Fragment")
            assert status == 200 and fragment.count(b"<article ") == 2, fragment[:200]
            assert b'card__meta' in fragment and f"{marker} card 00".encode() in fragment
            status, _, _ = request(target, board_path + "/involvement", "PUT", {"involvement": "access_only"}, bearer=True)
            assert status == 204, status
            assert "No one is watching for new cards" in html()
            status, _, _ = request(target, board_path + "/involvement", "PUT", {"involvement": "watching"}, bearer=True)
            assert status == 204 and "Watching for new cards" in html()
        explicit_default = html("?indexed_by=all&sorted_by=latest")
        assert GRID not in explicit_default
        tagged = html("?tag_ids%5B%5D=" + urllib.parse.quote(tag_id))
        assert GRID in tagged and f"{marker} card 00" in tagged and f"{marker} card 01" not in tagged
        if target["name"] == "rizzy":
            assert 'name="tag_ids[]"' in plain and f'value="{tag_id}"' in plain
            assert re.search(rf'name="tag_ids\[\]" value="{re.escape(tag_id)}" checked', tagged)
        closed = html("?indexed_by=closed")
        assert GRID in closed and special["closed"] in closed and special["not_now"] not in closed
        postponed = html("?indexed_by=not_now")
        assert GRID in postponed and special["not_now"] in postponed and special["closed"] not in postponed
        term = urllib.parse.quote(marker)
        first = html(f"?terms%5B%5D={term}")
        second = html(f"?terms%5B%5D={term}&page=2")
        assert GRID in first and GRID in second
        first_numbers = [index for index in range(17) if f"{marker} card {index:02d}" in first]
        second_numbers = [index for index in range(17) if f"{marker} card {index:02d}" in second]
        assert first_numbers == list(range(2, 17)) and second_numbers == [0, 1], (first_numbers, second_numbers)
        assert special["closed"] not in first and special["not_now"] not in first
        oldest = html("?sorted_by=oldest")
        assert GRID in oldest and f"{marker} card 00" in oldest and f"{marker} card 16" not in oldest
        empty = html("?terms%5B%5D=notpresentzzzz")
        assert "No cards match this filter" in empty
        status, _, _ = request(target, f"/{account}/cards/{tagged_number}/self_assignment.json",
                               "POST", b"", bearer=True)
        assert status == 204, (target["name"], "self assignment", status)
        owner = urllib.parse.quote(owner_id)
        assigned = html("?assignee_ids%5B%5D=" + owner)
        assert f"{marker} card 00" in assigned and f"{marker} card 01" not in assigned
        unassigned = html("?assignment_status=unassigned")
        assert f"{marker} card 00" not in unassigned and f"{marker} card 02" in unassigned
        created_by = html("?creator_ids%5B%5D=" + owner)
        assert f"{marker} card 00" in created_by
        closed_by = html("?closer_ids%5B%5D=" + owner)
        assert special["closed"] in closed_by and special["not_now"] not in closed_by
        if target["name"] == "rizzy":
            for field, page in (("assignee_ids[]", assigned), ("creator_ids[]", created_by),
                                ("closer_ids[]", closed_by)):
                assert f'name="{field}"' in plain
                assert re.search(rf'name="{re.escape(field)}" value="{re.escape(owner_id)}" checked', page)
            assert 'name="assignment_status" value="unassigned" checked' in unassigned
        return {"columns_without_filter": True, "special_filters": True, "tag_filter": True,
                "term_pages": (first_numbers, second_numbers), "oldest": True, "empty": True,
                "people_filters": True}
    finally:
        status, _, _ = request(target, board_path, "DELETE", bearer=True)
        assert status == 204, status


def main():
    fizzy_db = os.environ.get("FIZZY_DB_PATH", "upstream/storage/production.sqlite3")
    with sqlite3.connect(fizzy_db) as db:
        email = db.execute(
            "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (int(os.environ["FIZZY_ACCOUNT"]),),
        ).fetchone()[0]
    targets = []
    for name in ("FIZZY", "RIZZY"):
        target = {"name": name.lower(), "url": os.environ[f"{name}_URL"],
                  "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
                  "email": email if name == "FIZZY" else os.environ.get("RIZZY_EMAIL", "owner@example.com"),
                  "db": fizzy_db}
        if name == "FIZZY":
            # Isolate this HTML test from other suites' local sign-in rate limit.
            target["extra_headers"] = {"X-Forwarded-For": f"198.51.100.{time.time_ns() % 200 + 30}"}
        target["opener"] = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
        targets.append(target)
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    print("Board HTML filters matched: columns, state, terms, pagination, sorting, and empty results")


if __name__ == "__main__":
    main()
