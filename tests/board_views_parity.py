#!/usr/bin/env python3
"""Compare Fizzy and Rizzy's private and public board column views."""
import json
import os
import time
import urllib.error
import urllib.request


def request(target, path, method="GET", payload=None, accept="application/json", public=False):
    url = f"{target['url']}/{target['account']}{path}"
    headers = {"Accept": accept, "Content-Type": "application/json"}
    if not public:
        headers["Authorization"] = f"Bearer {target['token']}"
    req = urllib.request.Request(url, data=json.dumps(payload).encode() if payload is not None else None, method=method, headers=headers)
    try:
        response = urllib.request.urlopen(req, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        body = response.read()
        return response.status, json.loads(body) if accept == "application/json" and body else body.decode(errors="replace"), response.headers


def shape(value):
    if isinstance(value, dict):
        return {key: shape(item) for key, item in value.items()}
    if isinstance(value, list):
        return [shape(value[0])] if value else []
    return type(value).__name__


def run(target):
    prefix = f"View parity {time.time_ns()}"
    code, board, _ = request(target, "/boards", "POST", {"board": {"name": prefix}})
    assert code == 201
    board_id = board["id"]
    code, column, _ = request(target, f"/boards/{board_id}/columns", "POST", {"column": {"name": "Doing"}})
    assert code == 201
    cards = {}
    for state in ("maybe", "gold", "column", "column gold", "not_now", "closed"):
        code, card, _ = request(target, f"/boards/{board_id}/cards", "POST", {"card": {"title": f"Parity card {state}"}})
        assert code == 201
        cards[state] = card
    for state in ("gold", "column gold"):
        code, _, _ = request(target, f"/cards/{cards[state]['number']}/goldness", "POST")
        assert code == 204
    for state in ("column", "column gold"):
        code, _, _ = request(target, f"/cards/{cards[state]['number']}/triage", "POST", {"column_id": column["id"]})
        assert code == 204
    code, _, _ = request(target, f"/cards/{cards['not_now']['number']}/not_now", "POST")
    assert code == 204
    code, _, _ = request(target, f"/cards/{cards['closed']['number']}/closure", "POST")
    assert code == 204
    code, published, _ = request(target, f"/boards/{board_id}/publication", "POST")
    assert code == 201
    key = published["public_url"].split("/public/boards/")[1]

    result = {}
    expected = {"stream": ["Parity card gold", "Parity card maybe"], "not_now": ["Parity card not_now"], "closed": ["Parity card closed"]}
    for view, names in expected.items():
        code, payload, headers = request(target, f"/boards/{board_id}/columns/{view}")
        assert code == 200 and [item["title"] for item in payload] == names, (view, payload)
        result[f"private_{view}"] = (code, shape(payload), names)
        code, html, headers = request(target, f"/public/boards/{key}/columns/{view}", accept="text/html", public=True)
        assert code == 200
        for state in cards:
            title = f"Parity card {state}"
            assert (title in html) == (title in names), (view, title)
        result[f"public_{view}"] = (code, names)

    code, payload, _ = request(target, f"/boards/{board_id}/columns/{column['id']}/cards")
    assert code == 200 and [item["title"] for item in payload] == ["Parity card column gold", "Parity card column"]
    result["private_column"] = (code, shape(payload), [item["title"] for item in payload])
    code, html, _ = request(target, f"/public/boards/{key}/columns/{column['id']}", accept="text/html", public=True)
    assert code == 200 and "Parity card column gold" in html and "Parity card column" in html
    result["public_column"] = (code, "column gold" in html and "column" in html)
    code, html, _ = request(target, f"/public/boards/{key}", accept="text/html", public=True)
    assert code == 200 and "/columns/stream" in html and "/columns/closed" in html and "/columns/not_now" in html
    result["public_navigation"] = code

    code, _, _ = request(target, f"/public/boards/invalid-key/columns/stream", accept="text/html", public=True)
    assert code == 404
    result["missing_publication"] = code
    code, _, _ = request(target, f"/boards/{board_id}/columns/stream", accept="application/json", public=True)
    assert code != 200
    result["private_requires_auth"] = code != 200

    for index in range(17):
        code, _, _ = request(target, f"/boards/{board_id}/cards", "POST", {"card": {"title": f"Paged stream card {index:02d}"}})
        assert code == 201
    code, first_page, headers = request(target, f"/boards/{board_id}/columns/stream?page=1")
    assert code == 200 and len(first_page) == 15 and first_page[0]["title"] == "Parity card gold"
    assert 'rel="next"' in headers.get("Link", "")
    code, second_page, _ = request(target, f"/boards/{board_id}/columns/stream?page=2")
    assert code == 200 and len(second_page) == 4 and second_page[-1]["title"] == "Parity card maybe"
    result["private_pagination"] = (len(first_page), len(second_page), bool(headers.get("Link")))
    code, public_first, _ = request(target, f"/public/boards/{key}/columns/stream?page=1", accept="text/html", public=True)
    assert code == 200 and "Paged stream card 16" in public_first and "Paged stream card 00" not in public_first
    assert 'id="stream_column-pagination-link-2"' in public_first
    assert 'aria-label="Load page 2"' in public_first
    assert f"/public/boards/{key}/columns/stream?page=2" in public_first
    code, public_second, _ = request(target, f"/public/boards/{key}/columns/stream?page=2", accept="text/html", public=True)
    assert code == 200 and "Paged stream card 00" in public_second and "Parity card maybe" in public_second
    assert 'id="stream_column-pagination-link-3"' not in public_second
    result["public_pagination"] = (code, "Paged stream card 00" in public_second)
    code, _, _ = request(target, f"/boards/{board_id}", "DELETE")
    assert code == 204
    return result


def main():
    targets = []
    for prefix in ("FIZZY", "RIZZY"):
        targets.append({"url": os.environ[prefix + "_URL"], "account": os.environ[prefix + "_ACCOUNT"], "token": os.environ[prefix + "_TOKEN"]})
    source, rust = (run(target) for target in targets)
    assert source == rust, {key: (source.get(key), rust.get(key)) for key in source.keys() | rust.keys() if source.get(key) != rust.get(key)}
    print(f"Board column views matched: {len(source)} private/public status, shape, order, and visibility checks")


if __name__ == "__main__":
    main()
