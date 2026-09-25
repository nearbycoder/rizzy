#!/usr/bin/env python3
"""Compare plain-text search of formatted card/comment content and embeds."""
import json
import os
import sqlite3
import time
import urllib.parse

from card_image_parity import call
from embed_parity import FILE, upload


def request(target, path, method="GET", payload=None, expected=200):
    status, _, body = call(target, f"/{target['account']}{path}", method, payload)
    assert status == expected, (method, path, status, body[:160])
    return json.loads(body) if body else None


def run(target):
    unique = "plainsearch" + str(time.time_ns())
    board = request(target, "/boards", "POST", {"board": {"name": "Search text " + unique}}, 201)
    board_id = board["id"]
    values = {}
    try:
        markup = f'<p class="{unique}shadow">A <strong>{unique}brisk</strong> route &amp; lake</p>'
        card = request(target, f"/boards/{board_id}/cards", "POST",
                       {"card": {"title": "Neutral", "description": markup}}, 201)
        number = card["number"]
        comment = request(target, f"/cards/{number}/comments", "POST",
                          {"comment": {"body": f'<div class="{unique}ghost"><em>{unique}tidy</em></div>'}}, 201)

        def matched(term, number=number):
            search = request(target, "/search?q=" + urllib.parse.quote(term))
            params = urllib.parse.urlencode({"board_ids[]": board_id, "terms[]": term})
            filtered = request(target, "/cards?" + params)
            return (number in [row["number"] for row in search],
                    number in [row["number"] for row in filtered])

        for label, term in (("visible", unique + "brisk"), ("tag", "strong"),
                            ("attribute", unique + "shadow"), ("entity", "amp"),
                            ("comment_visible", unique + "tidy"), ("comment_attribute", unique + "ghost")):
            values[label] = matched(term)

        request(target, f"/cards/{number}.json", "PATCH",
                {"card": {"description": f"<p>{unique}renewed</p>"}})
        values["old_card_term"] = matched(unique + "brisk")
        values["new_card_term"] = matched(unique + "renewed")
        request(target, f"/cards/{number}/comments/{comment['id']}", "PATCH",
                {"comment": {"body": f"<p>{unique}changed</p>"}})
        values["old_comment_term"] = matched(unique + "tidy")
        values["new_comment_term"] = matched(unique + "changed")

        filename_term = unique + "filemark"
        blob = upload(target, FILE, filename_term + ".txt", "text/plain")
        embedded = request(target, f"/boards/{board_id}/cards", "POST",
                           {"card": {"title": "File", "description":
                                     f'<p><action-text-attachment sgid="{blob["attachable_sgid"]}"></action-text-attachment></p>'}}, 201)
        values["filename"] = matched(filename_term, embedded["number"])
        values["markup_name"] = matched("sgid", embedded["number"])

        # The source serializer caps indexed content at 32 KiB.
        late_term = unique + "late"
        long_card = request(target, f"/boards/{board_id}/cards", "POST",
                            {"card": {"title": "Long", "description":
                                      unique + "early " + ("filler " * 5000) + late_term}}, 201)
        values["long_early"] = matched(unique + "early", long_card["number"])
        values["long_late"] = matched(late_term, long_card["number"])

        request(target, f"/cards/{number}/comments/{comment['id']}", "DELETE", expected=204)
        values["deleted_comment"] = matched(unique + "changed")
        request(target, f"/cards/{number}", "DELETE", expected=204)
        values["deleted_card"] = matched(unique + "renewed")

        if target.get("database"):
            with sqlite3.connect(target["database"]) as db:
                indexed = db.execute("SELECT search_text FROM cards WHERE id=?", (embedded["id"],)).fetchone()[0]
                assert indexed == f"[{filename_term}.txt]", indexed
                assert db.execute("SELECT length(search_text) FROM cards WHERE id=?",
                                  (long_card["id"],)).fetchone()[0] <= 32768
        return values
    finally:
        request(target, f"/boards/{board_id}", "DELETE", expected=204)


def main():
    results = {}
    for name in ("FIZZY", "RIZZY"):
        if not os.environ.get(f"{name}_URL"):
            continue
        target = {"url": os.environ[f"{name}_URL"], "account": os.environ[f"{name}_ACCOUNT"],
                  "token": os.environ[f"{name}_TOKEN"]}
        if name == "RIZZY" and os.environ.get("RIZZY_DB_PATH"):
            target["database"] = os.environ["RIZZY_DB_PATH"]
        results[name] = run(target)
    if len(results) == 2:
        assert results["FIZZY"] == results["RIZZY"], results
    for result in results.values():
        for label in ("visible", "comment_visible", "new_card_term", "new_comment_term",
                      "filename", "long_early"):
            assert result[label] == (True, True), (label, result[label])
        for label in ("tag", "entity", "attribute", "comment_attribute", "old_card_term", "old_comment_term",
                      "markup_name", "long_late", "deleted_comment", "deleted_card"):
            assert result[label] == (False, False), (label, result[label])
    print(f"Formatted search parity passed on {', '.join(results)}")


if __name__ == "__main__":
    main()
