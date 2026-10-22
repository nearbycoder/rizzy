#!/usr/bin/env python3
"""Compare browser search hits, snippets, and pagination with pinned Fizzy."""
import http.cookiejar
import json
import os
import re
import sqlite3
import subprocess
import tempfile
import time
import urllib.parse
import urllib.request
from html import unescape
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from draft_workflow_parity import NoRedirect, login, request


def source_target():
    required = ("FIZZY_URL", "FIZZY_ACCOUNT", "FIZZY_TOKEN", "FIZZY_DB")
    missing = [key for key in required if not os.getenv(key)]
    if missing:
        raise SystemExit("Set " + ", ".join(missing))
    with sqlite3.connect(os.environ["FIZZY_DB"]) as db:
        row = db.execute(
            "SELECT i.email_address FROM identities i JOIN users u ON u.identity_id=i.id "
            "JOIN accounts a ON a.id=u.account_id WHERE a.external_account_id=? "
            "AND u.role='owner' AND u.active=1 LIMIT 1", (os.environ["FIZZY_ACCOUNT"],)
        ).fetchone()
    assert row, "source owner not found"
    return {
        "name": "fizzy", "url": os.environ["FIZZY_URL"],
        "account": os.environ["FIZZY_ACCOUNT"], "token": os.environ["FIZZY_TOKEN"],
        "db": os.environ["FIZZY_DB"], "email": row[0],
        "extra_headers": {"X-Forwarded-For": "198.51.100." + str(30 + time.time_ns() % 200)},
        "opener": urllib.request.build_opener(NoRedirect(), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())),
    }


def rust_target(temp, *, release=False):
    db = Path(temp) / "search-html.sqlite3"
    email = "search-html@example.com"
    env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{db}",
           "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
           "RIZZY_SEED_EMAIL": email}
    subprocess.run(["cargo", "build", "--release" if release else "--quiet"], cwd=ROOT, check=True)
    binary = ROOT / "target/release/rizzy" if release else BINARY
    seeded = subprocess.run([binary, "seed"], cwd=ROOT, env=env,
                            capture_output=True, text=True, check=True)
    account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
    token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
    port, process = start(env, binary=binary)
    return ({
        "name": "rizzy", "url": f"http://127.0.0.1:{port}",
        "account": account, "token": token, "db": str(db), "email": email,
        "opener": urllib.request.build_opener(NoRedirect(), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())),
    }, process)


def api(target, path, method="GET", data=None, expected=200):
    status, _, raw = request(target, f"/{target['account']}{path}", method, data, bearer=True)
    assert status == expected, (target["name"], method, path, status, raw[:200])
    return json.loads(raw) if raw else None


def html(target, term, page=None):
    params = {"q": term}
    if page is not None:
        params["page"] = page
    status, _, raw = request(target, f"/{target['account']}/search?{urllib.parse.urlencode(params)}",
                             accept="text/html")
    assert status == 200, (target["name"], term, page, status, raw[:200])
    return raw.decode()


def titles(document):
    return [unescape(re.sub(r"<[^>]+>", "", value)).strip() for value in re.findall(
        r'<h3[^>]*class="[^"]*search__title[^"]*"[^>]*>(.*?)</h3>', document, re.S)]


def card_excerpts(document):
    return [unescape(re.sub(r"<[^>]+>", "", value)).strip() for value in re.findall(
        r'<div class="search__excerpt">(.*?)</div>', document, re.S)]


def check(target, tag):
    login(target)
    board = api(target, "/boards", "POST", {"board": {"name": "Search HTML parity " + tag}}, 201)
    board_id = board["id"]
    try:
        def new_card(title, description=""):
            return api(target, f"/boards/{board_id}/cards", "POST",
                       {"card": {"title": title, "description": description}}, 201)

        first = new_card("Layout " + tag + " broken" + tag, "Look at this " + tag + " mess")
        second = new_card("Some card " + tag)
        api(target, f"/cards/{second['number']}/comments", "POST",
            {"comment": {"body": "overflowing" + tag + " text issue"}}, 201)
        both = new_card("Just " + tag, "More " + tag)
        api(target, f"/cards/{both['number']}/comments", "POST",
            {"comment": {"body": "I love " + tag}}, 201)
        escaped = new_card("<b>Bold</b> " + tag + " testing" + tag)
        new_card("Fallback titlehit" + tag, "word " * 60)
        filler = " ".join(["word"] * 20)
        long_card = new_card("Long winded " + tag, f"{filler} needle{tag} {filler}")
        ascii_card = new_card("caf " + tag)
        new_card("café " + tag)
        quote_card = new_card("gamma" + tag + " 123 beta")
        for i in range(20):
            new_card(f"page{tag} card {i}")

        landing = html(target, "")
        assert re.search(r'<h1[^>]*class="[^"]*header__title[^"]*"[^>]*>Search</h1>', landing)
        assert not titles(landing) and 'class="search__blank-slate' not in landing
        card_doc = html(target, "broken" + tag)
        assert any("Layout" in title and "broken" in title for title in titles(card_doc))
        assert "Look at this" in card_doc
        comment_doc = html(target, "overflowing" + tag)
        assert any("Some card" in title for title in titles(comment_doc))
        assert 'class="search__excerpt search__excerpt--comment' in comment_doc and "text issue" in comment_doc
        assert re.search(
            rf'<span class="avatar"[^>]*><img [^>]*src="/{target["account"]}/users/[^"/]+/avatar"[^>]*/?></span>',
            comment_doc,
        ), target["name"]
        both_doc = html(target, tag)
        assert sum("Just " + tag in title for title in titles(both_doc)) == 2
        assert re.search(r'<mark class="circled-text"><span></span>' + tag + '</mark>', both_doc, re.I)
        assert 'class="search__excerpt search__excerpt--comment' in both_doc
        assert 'data-controller="auto-submit"' in html(target, str(first["number"]))
        assert 'data-controller="auto-submit"' not in html(target, "999999999")
        assert "No matches" in html(target, "999999999")
        escaped_doc = html(target, "testing" + tag)
        assert "&lt;b&gt;Bold&lt;/b&gt;" in escaped_doc
        assert "<mark class=\"circled-text\"><span></span>testing" + tag + "</mark>" in escaped_doc
        long_doc = html(target, "needle" + tag)
        assert any("Long winded" in title for title in titles(long_doc))
        assert "needle" + tag in long_doc and "..." in long_doc
        fallback_doc = html(target, "titlehit" + tag)
        assert len(card_excerpts(fallback_doc)) == 1
        accent_doc = html(target, "café " + tag)
        assert len(titles(accent_doc)) == 1
        assert str(ascii_card["number"]) in titles(accent_doc)[0]
        assert '<mark class="circled-text"><span></span>caf</mark>' in accent_doc
        assert not titles(html(target, "東京"))
        assert str(quote_card["number"]) in titles(html(target, "gamma" + tag + " 123 beta"))[0]
        quoted_term = '123(gamma' + tag + '"beta|"'
        assert not titles(html(target, quoted_term))
        assert not api(target, "/search?" + urllib.parse.urlencode({"q": quoted_term}))
        first_page = titles(html(target, "page" + tag))
        second_page = titles(html(target, "page" + tag, 2))
        assert len(first_page) == 15 and len(second_page) == 5
        assert not set(first_page) & set(second_page)
        assert 'id="filtered_search_results-pagination-link-2"' in html(target, "page" + tag)
        assert 'id="filtered_search_results-pagination-link-3"' not in html(target, "page" + tag, 2)
        for value in ("0", "-1", "abc"):
            assert len(titles(html(target, "page" + tag, value))) == 15
        assert len(titles(html(target, "page" + tag, 5000))) <= 15
        json_hits = api(target, "/search?q=" + tag)
        assert len([card for card in json_hits if card["id"] == both["id"]]) == 1
        return {"card": len(titles(card_doc)), "comment": len(titles(comment_doc)),
                "double_hit": sum("Just " + tag in title for title in titles(both_doc)),
                "page_counts": [len(first_page), len(second_page)],
                "card_excerpt": card_excerpts(card_doc),
                "long_excerpt": card_excerpts(long_doc),
                "fallback_excerpt": card_excerpts(fallback_doc),
                "accent_titles": len(titles(accent_doc)),
                "escaped_card": escaped["number"] > 0, "long_card": long_card["number"] > 0}
    finally:
        api(target, f"/boards/{board_id}", "DELETE", expected=204)


def main():
    source = source_target()
    with tempfile.TemporaryDirectory() as temp:
        rust, process = rust_target(temp)
        try:
            tag = "search" + str(time.time_ns())
            expected = check(source, tag)
            actual = check(rust, tag)
            assert actual == expected, (expected, actual)
            print("Browser search parity passed on Fizzy and Rizzy:", actual)
        finally:
            stop(process)


if __name__ == "__main__":
    main()
