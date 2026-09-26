#!/usr/bin/env python3
"""Compare common Action Text fields and safe HTML rendering."""
import json
import os
import time

from card_image_parity import call


CASES = [
    "",
    "Plain text",
    '<p>Hello <strong>world</strong> &amp; <a href="https://example.com">link</a></p>',
    "<p>Next<br>line</p><ul><li>One</li><li>Two</li></ul>",
    "<script>alert(1)</script><p>Safe</p>",
    '<p><a href="javascript:alert(1)" data-action="x">bad</a> <em>good</em></p>',
    "First\nsecond",
    "<p>One</p><p>Two</p>",
    "<div>Hello</div><div>World</div>",
    "See https://example.com now",
    '<p><a href="https://example.com">https://example.com</a> <code>https://example.com</code></p>',
    '<p class="note">Styled <strong>content</strong></p>',
]


def request(target, path, method="GET", payload=None):
    status, _, body = call(target, f"/{target['account']}{path}", method, payload)
    assert status in (200, 201, 204), (method, path, status, body[:150])
    return json.loads(body) if body else None


def run(target):
    board = request(target, "/boards", "POST", {"board": {
        "name": f"Rich text parity {time.time_ns()}",
        "public_description": "<p>Welcome <strong>home</strong></p>",
    }})
    result = {"cards": []}
    try:
        first = None
        for value in CASES:
            card = request(target, f"/boards/{board['id']}/cards", "POST",
                           {"card": {"title": "Rich text", "description": value}})
            result["cards"].append((card["description"], card["description_html"]))
            if first is None:
                first = card
        updated = request(target, f"/cards/{first['number']}.json", "PATCH",
                          {"card": {"description": "<h2>Header</h2><p>Body</p>"}})
        result["updated"] = (updated["description"], updated["description_html"])
        cleared = request(target, f"/cards/{first['number']}.json", "PATCH",
                          {"card": {"description": ""}})
        result["cleared"] = (cleared["description"], cleared["description_html"])
        comment = request(target, f"/cards/{first['number']}/comments", "POST",
                          {"comment": {"body": "<p>Look <em>here</em> &amp; email me@example.com</p>"}})
        result["comment"] = (comment["body"]["plain_text"], comment["body"]["html"])
        published = request(target, f"/boards/{board['id']}/publication", "POST")
        result["board"] = (published.get("public_description"), published.get("public_description_html"))
        return result
    finally:
        request(target, f"/boards/{board['id']}", "DELETE")


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        if not os.environ.get(f"{name}_URL"):
            continue
        results[name] = run({"url": os.environ[f"{name}_URL"],
                             "account": os.environ[f"{name}_ACCOUNT"],
                             "token": os.environ[f"{name}_TOKEN"]})
    if len(results) == 2:
        source, rust = results["FIZZY"], results["RIZZY"]
        differences = {key: (source[key], rust[key]) for key in source if source[key] != rust[key]}
        assert not differences, json.dumps(differences, indent=2)
    print(f"Rich text fields matched on {', '.join(results)}")


if __name__ == "__main__":
    main()
