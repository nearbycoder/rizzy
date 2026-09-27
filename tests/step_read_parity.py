#!/usr/bin/env python3
"""Compare step detail/edit reads and the edited value against Fizzy."""
import http.cookiejar
import json
import os
import re
import sqlite3
import time
import urllib.request

from draft_workflow_parity import NoRedirect, login, request


def checked(target, path, method="GET", payload=None, *, accept="application/json", bearer=True, expected=200):
    status, _, body = request(target, f"/{target['account']}{path}", method, payload,
                              accept=accept, bearer=bearer)
    assert status == expected, (method, path, status, body[:180])
    return json.loads(body) if accept == "application/json" and status in (200, 201) else body


def run(target):
    login(target)
    board = checked(target, "/boards", "POST", {"board": {"name": f"Step read parity {time.time_ns()}"}}, expected=201)
    try:
        card = checked(target, f"/boards/{board['id']}/cards", "POST",
                       {"card": {"title": "Step read probe"}}, expected=201)
        number = card["number"]
        step = checked(target, f"/cards/{number}/steps", "POST",
                       {"step": {"content": "First <task> & check"}}, expected=201)
        path = f"/cards/{number}/steps/{step['id']}"
        value = checked(target, path + ".json")
        assert value == {"id": step["id"], "content": "First <task> & check", "completed": False}
        checked(target, path + "/edit.json", expected=406)

        card_page = checked(target, f"/cards/{number}", accept="text/html", bearer=False).decode()
        assert re.search(rf'<a\b[^>]*href="/{target["account"]}{re.escape(path)}/edit"[^>]*>First &lt;task&gt; &amp; check</a>', card_page), card_page[:500]

        show = checked(target, path, accept="text/html", bearer=False).decode()
        edit = checked(target, path + "/edit", accept="text/html", bearer=False).decode()
        assert f'id="step_{step["id"]}"' in show and "First &lt;task&gt; &amp; check" in show
        assert f"{path}/edit" in show
        assert f'id="step_{step["id"]}"' in edit
        content_fields = re.findall(r"<input\b[^>]*>", edit)
        assert any('name="step[content]"' in field and 'value="First &lt;task&gt; &amp; check"' in field
                   for field in content_fields), edit[max(0, edit.find('First &lt;task') - 260):edit.find('First &lt;task') + 200]
        assert "Save changes" in edit and "Delete this step" in edit

        edited = checked(target, path + ".json", "PATCH",
                         {"step": {"content": "Revised task", "completed": True}})
        assert edited == {"id": step["id"], "content": "Revised task", "completed": True}
        assert checked(target, path + ".json") == edited
        edit = checked(target, path + "/edit", accept="text/html", bearer=False).decode()
        assert 'value="Revised task"' in edit and re.search(r'type="checkbox"[^>]*checked', edit)
        checked(target, f"/cards/{number}/steps/missing-step.json", expected=404)
        return {"detail_json": True, "edit_json_406": True, "card_edit_link": True, "show_html": True,
                "edit_html": True, "edited_value": True, "missing_404": True}
    finally:
        checked(target, f"/boards/{board['id']}", "DELETE", expected=204)


def main():
    fizzy_db = os.environ.get("FIZZY_DB_PATH", "upstream/storage/production.sqlite3")
    with sqlite3.connect(fizzy_db) as db:
        fizzy_email = db.execute(
            "SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
            "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
            (int(os.environ["FIZZY_ACCOUNT"]),)
        ).fetchone()[0]
    results = []
    for name in ("FIZZY", "RIZZY"):
        target = {"name": name.lower(), "url": os.environ[f"{name}_URL"],
                  "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"],
                  "email": fizzy_email if name == "FIZZY" else os.environ.get("RIZZY_EMAIL", "owner@example.com"),
                  "db": fizzy_db,
                  "opener": urllib.request.build_opener(
                      urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
        results.append(run(target))
    assert results[0] == results[1], results
    print("Step detail and edit parity passed on FIZZY, RIZZY")


if __name__ == "__main__":
    main()
