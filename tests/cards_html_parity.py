#!/usr/bin/env python3
"""Compare all-cards browser filtering and page boundaries with Fizzy."""
import http.cookiejar
import json
import os
import sqlite3
import time
import urllib.parse
import urllib.request

from draft_workflow_parity import NoRedirect, login, request


def check(target, signed_in=False):
    if not signed_in:
        login(target)
    account = target["account"]
    marker = f"cardshtml{time.time_ns()}"
    status, _, raw = request(target, f"/{account}/boards", "POST", {"board": {"name": marker}}, bearer=True)
    assert status == 201, (target["name"], status, raw[:160])
    board_id = json.loads(raw)["id"]
    second_board_id = None
    try:
        for index in range(17):
            status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                     {"card": {"title": f"{marker} card {index:02d}"}}, bearer=True)
            assert status == 201, (target["name"], index, status, raw[:160])
            if index == 16:
                newest = json.loads(raw)
        number = newest["number"]
        status, _, raw = request(target, f"/{account}/cards/{number}/steps", "POST",
                                 {"step": {"content": "Preview parity step"}}, bearer=True)
        assert status == 201, (target["name"], status, raw[:160])
        step_id = json.loads(raw)["id"]
        status, _, _ = request(target, f"/{account}/cards/{number}/steps/{step_id}", "PUT",
                               {"step": {"completed": True}}, bearer=True)
        assert status == 200, (target["name"], status)
        status, _, _ = request(target, f"/{account}/cards/{number}/taggings", "POST",
                               {"tag_title": "#preview-parity"}, bearer=True)
        assert status == 204, (target["name"], status)
        assignee_id = newest["creator"]["id"]
        status, _, _ = request(target, f"/{account}/cards/{number}/assignments", "POST",
                               {"assignee_id": assignee_id}, bearer=True)
        assert status == 204, (target["name"], status)
        status, _, raw = request(target, f"/{account}/boards/{board_id}/cards", "POST",
                                 {"card": {"title": f"{marker} completed"}}, bearer=True)
        assert status == 201, (target["name"], status)
        number = json.loads(raw)["number"]
        status, _, raw = request(target, f"/{account}/cards/{number}/closure", "POST", bearer=True)
        assert status == 204, (target["name"], status, raw[:160])
        base = f"/{account}/cards?" + urllib.parse.urlencode({"board_ids[]": board_id})

        def html(suffix=""):
            status, headers, raw = request(target, base + suffix, accept="text/html")
            assert status == 200 and headers.get_content_type() == "text/html", (target["name"], suffix, status, raw[:160])
            body = raw.decode()
            assert marker in body and "cards--grid" in body and "cards_container" in body
            return body

        def found(body):
            return [index for index in range(17) if f"{marker} card {index:02d}" in body]

        first_body = html()
        assert 'id="filter-settings-save-toggle"' not in first_body, target["name"]
        first = found(first_body)
        assert "preview-parity" in first_body and "1/1" in first_body, target["name"]
        second = found(html("&page=2"))
        closed = html("&indexed_by=closed")
        term = found(html("&terms%5B%5D=" + urllib.parse.quote(marker + " card 05")))
        oldest = found(html("&sorted_by=oldest"))
        assigned = found(html("&assignee_ids%5B%5D=" + urllib.parse.quote(assignee_id)))
        unassigned_body = html("&assignment_status=unassigned")
        unassigned = found(unassigned_body)
        assert first == list(range(2, 17)), (target["name"], first)
        assert second == [0, 1], (target["name"], second)
        assert f"{marker} completed" in closed and not found(closed)
        expected_terms = list(range(2, 17)) if target.get("mysql") else [5]
        assert term == expected_terms, (target["name"], term)
        assert oldest == list(range(15)), (target["name"], oldest)
        assert assigned == [16], (target["name"], assigned)
        assert unassigned == list(range(1, 16)), (target["name"], unassigned)
        json_path = f"/{account}/cards.json?board_ids%5B%5D={urllib.parse.quote(board_id)}"
        status, _, raw = request(target, json_path, bearer=True)
        assert status == 200 and len(json.loads(raw)) == 15, (target["name"], status)
        status, _, raw = request(target, f"/{account}/boards", "POST",
                                 {"board": {"name": marker + " second"}}, bearer=True)
        assert status == 201, (target["name"], status, raw[:160])
        second_board_id = json.loads(raw)["id"]
        status, _, raw = request(target, f"/{account}/boards/{second_board_id}/cards", "POST",
                                 {"card": {"title": "Access order probe"}}, bearer=True)
        assert status == 201, (target["name"], status, raw[:160])
        second_number = json.loads(raw)["number"]
        for read_number in (newest["number"], second_number):
            status, _, raw = request(target, f"/{account}/cards/{read_number}/reading", "POST", bearer=True)
            assert status == 201, (target["name"], status, raw[:160])
            time.sleep(0.02)
        ordered_html = html()
        if target["name"] == "fizzy":
            choice = lambda board: f'data-multi-selection-combobox-value="{board}"'
        else:
            choice = lambda board: f'name="board_ids[]" value="{board}"'
        first_position = ordered_html.find(choice(board_id))
        second_position = ordered_html.find(choice(second_board_id))
        assert second_position >= 0 and first_position > second_position, (
            target["name"], "board choices are not ordered by recent access", first_position, second_position)
        return first, second, term, oldest, assigned, unassigned
    finally:
        if second_board_id:
            status, _, _ = request(target, f"/{account}/boards/{second_board_id}", "DELETE", bearer=True)
            assert status == 204, (target["name"], status)
        status, _, _ = request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
        assert status == 204, (target["name"], status)


def main():
    source_db = os.environ.get("FIZZY_DB_PATH", "upstream/storage/production.sqlite3")
    with sqlite3.connect(source_db) as db:
        email = db.execute(
            "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (int(os.environ["FIZZY_ACCOUNT"]),),
        ).fetchone()[0]
    targets = []
    for name in ("FIZZY", "RIZZY"):
        target = {"name": name.lower(), "url": os.environ[f"{name}_URL"],
                  "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
                  "email": email if name == "FIZZY" else os.environ["RIZZY_EMAIL"],
                  "db": source_db,
                  "opener": urllib.request.build_opener(
                      urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
        if name == "FIZZY":
            target["extra_headers"] = {"X-Forwarded-For": f"198.51.100.{30 + time.time_ns() % 200}"}
        targets.append(target)
    left, right = (check(target) for target in targets)
    assert left == right, (left, right)
    print("All-cards HTML parity passed: pages, filters, recent board order, and JSON")


if __name__ == "__main__":
    main()
