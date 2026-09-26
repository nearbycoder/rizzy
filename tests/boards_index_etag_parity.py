#!/usr/bin/env python3
"""Compare conditional board index responses on live Fizzy and Rizzy."""
import os
import json
import urllib.error
import urllib.request

from activity_event_board_parity import call
from boards_index_order_parity import cleanup, fixture, verify
from parity import shape


def request(target, page, etag=None):
    headers = {"Authorization": f"Bearer {target['token']}", "Accept": "application/json"}
    if etag:
        headers["If-None-Match"] = etag
    url = f"{target['url']}/{target['account']}/boards.json?page={page}"
    try:
        response = urllib.request.urlopen(urllib.request.Request(url, headers=headers), timeout=20)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def check(target, baseline):
    verify(target, baseline)
    status, headers, body = request(target, 4)
    assert status == 200 and body
    etag = headers.get("ETag")
    assert etag and etag.startswith('W/"')
    assert headers.get("Cache-Control") == "max-age=0, private, must-revalidate"
    assert headers.get("Content-Type") == "application/json; charset=utf-8"
    count, link = headers.get("X-Total-Count"), headers.get("Link")
    assert count and link and 'rel="next"' in link
    boards = json.loads(body)
    assert len(boards) == 100

    for conditional in (etag, f'W/"stale", {etag}', "*"):
        status, fresh, body = request(target, 4, conditional)
        assert status == 304 and body == b""
        assert fresh.get("ETag") == etag
        assert fresh.get("X-Total-Count") == count
        assert fresh.get("Link") == link
        assert fresh.get("Cache-Control") == headers.get("Cache-Control")
        assert fresh.get("Content-Type") is None
    assert request(target, 4, etag[2:])[0] == 200

    changed = "ETag changed board"
    call(target, f"/boards/{boards[0]['id']}", "PATCH", {"board": {"name": changed}}, 200)
    status, fresh, body = request(target, 4, etag)
    assert status == 200 and fresh.get("ETag") != etag
    assert json.loads(body)[0]["name"] == changed
    new_etag = fresh.get("ETag")
    assert request(target, 4, new_etag)[0] == 304

    first_page, _, first_body = request(target, 1)
    assert first_page == 200
    first = json.loads(first_body)[0]
    call(target, f"/boards/{first['id']}", "PATCH",
         {"board": {"name": "Off-page ETag change"}}, 200)
    assert request(target, 4, new_etag)[0] == 304
    return shape(boards[0])


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        fields = ("URL", "ACCOUNT", "TOKEN", "DB")
        if any(not os.getenv(f"{name}_{field}") for field in fields):
            raise SystemExit(f"Set {name}_URL, {name}_ACCOUNT, {name}_TOKEN, and {name}_DB")
        target = {"name": name.lower(),
                  **{field.lower(): os.environ[f"{name}_{field}"] for field in fields}}
        board, new_ids, baseline = fixture(target)
        try:
            results[name.lower()] = check(target, baseline)
        finally:
            cleanup(target, board, new_ids)
    assert results["fizzy"] == results["rizzy"], results
    print("Board index ETag, 304 headers, and page-specific invalidation matched Fizzy")


if __name__ == "__main__":
    main()
