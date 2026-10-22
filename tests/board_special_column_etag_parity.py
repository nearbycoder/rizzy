#!/usr/bin/env python3
"""Compare conditional JSON for Maybe, Not now, and Done board columns."""

import http.cookiejar
import os
import subprocess
import sys
import tempfile
import urllib.error
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, start_server,
                             stop_server)  # noqa: E402
from column_cards_etag_parity import request  # noqa: E402
from draft_workflow_parity import NoRedirect, login  # noqa: E402


def html_request(target, path, validator=None):
    headers = {"Accept": "text/html"}
    if validator:
        headers["If-None-Match"] = validator
    url = f"{target['url']}/{target['account']}{path}"
    try:
        response = target["opener"].open(urllib.request.Request(url, headers=headers), timeout=20)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def check(name, directory):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    if name == "rizzy":
        env["RIZZY_DEV_MAGIC_LINKS"] = "1"
    process, url = start_server(name, directory, env,
                                sorted(os.sched_getaffinity(0))[:2])
    try:
        target = {"name": name, "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"], "email": "owner@example.com",
                  "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                  "opener": urllib.request.build_opener(
                      urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
        login(target)
        board, _ = expect(target, "/boards", "POST",
                          {"board": {"name": "Special column caching"}}, 201)
        cards = []
        for title in ("Postpone this", "Close this"):
            card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                             {"card": {"title": title}}, 201)
            cards.append(card)
        paths = {name: f"/boards/{board['id']}/columns/{name}"
                 for name in ("stream", "not_now", "closed")}

        def snapshot():
            state = {}
            for kind, path in paths.items():
                status, headers, body = request(target, path)
                assert status == 200 and isinstance(body, list), (name, kind, status)
                tag = headers.get("ETag")
                assert tag and tag.startswith('W/"'), (name, kind, tag)
                assert headers.get("Cache-Control") == "max-age=0, private, must-revalidate"
                assert headers.get("Content-Type") == "application/json; charset=utf-8"
                assert headers.get("X-Total-Count") == str(len(body))
                assert request(target, path, tag)[0] == 304
                state[kind] = (tag, [item["number"] for item in body])
            return state

        initial = snapshot()
        assert initial["stream"][1] == [cards[1]["number"], cards[0]["number"]]
        assert initial["not_now"][1] == initial["closed"][1] == []
        for kind, path in paths.items():
            html_status, html_headers, html_body = html_request(target, path)
            assert html_status == 200 and html_body, (name, kind, html_status)
            html_etag = html_headers.get("ETag")
            assert html_etag and html_etag.startswith('W/"'), (name, kind, html_etag)
            cached_status, cached_headers, cached_body = html_request(target, path, html_etag)
            assert cached_status == 304 and cached_body == b"", (name, kind, cached_status)
            assert cached_headers.get("ETag") == html_etag
        expect(target, f"/cards/{cards[0]['number']}/not_now", "POST", None, 204)
        postponed = snapshot()
        assert postponed["stream"][1] == [cards[1]["number"]]
        assert postponed["not_now"][1] == [cards[0]["number"]]
        assert postponed["closed"] == initial["closed"], (name, initial, postponed)
        assert request(target, paths["closed"], initial["closed"][0])[0] == 304
        assert request(target, paths["stream"], initial["stream"][0])[0] == 200
        expect(target, f"/cards/{cards[1]['number']}/closure", "POST", None, 204)
        closed = snapshot()
        assert closed["stream"][1] == []
        assert closed["closed"][1] == [cards[1]["number"]]
        assert closed["not_now"] == postponed["not_now"], (name, postponed, closed)
        assert request(target, paths["not_now"], postponed["not_now"][0])[0] == 304
        return {key: (initial[key][1], postponed[key][1], closed[key][1]) for key in paths}
    finally:
        stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-special-column-etag-") as temporary:
        root = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = root / name
            directory.mkdir()
            results[name] = check(name, directory)
        assert results["fizzy"] == results["rizzy"], results
    print("Special column JSON validators matched pinned Fizzy")


if __name__ == "__main__":
    main()
