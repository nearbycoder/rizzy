#!/usr/bin/env python3
"""Compare anonymous published board and card pages with pinned Fizzy."""
import re
import tempfile
import time
import urllib.request

from draft_workflow_parity import request
from embed_parity import IMAGE, upload
from search_html_parity import api, rust_target, source_target
from auth import stop


def anonymous(target, path, expected=200):
    status, headers, raw = request(target, f"/{target['account']}{path}", accept="text/html")
    assert status == expected, (target["name"], path, status, raw[:200])
    return raw.decode(), headers


def column(document, section_id):
    section = re.search(r'<section id="' + re.escape(section_id) + r'"(.*?)</section>', document, re.S)
    assert section, section_id
    markup = section.group(0)
    count = re.search(r'<span class="cards__expander-count"[^>]*>(.*?)</span>', markup, re.S)
    frame = re.search(r'<turbo-frame[^>]*\bsrc="([^"]+)"', markup)
    assert count and frame, (section_id, markup[:200])
    return count.group(1).strip(), frame.group(1)


def meta(document, property_name):
    found = re.search(r'<meta property="' + re.escape(property_name) + r'" content="([^"]*)"', document)
    assert found, property_name
    return found.group(1)


def run(target, suffix):
    board_name = "Public parity " + suffix
    board = api(target, "/boards", "POST", {
        "board": {"name": board_name,
                  "public_description": "<p>Welcome <strong>visitors</strong></p>"}}, 201)
    board_id = board["id"]
    try:
        named = api(target, f"/boards/{board_id}/columns", "POST",
                    {"column": {"name": "Doing"}}, 201)

        def card(title, description=""):
            return api(target, f"/boards/{board_id}/cards", "POST",
                       {"card": {"title": title, "description": description}}, 201)

        maybe = card("Maybe " + suffix, "<p>Body <strong>bold</strong></p>")
        doing = card("Doing " + suffix)
        not_now = card("Not now " + suffix)
        done = card("Done " + suffix)
        first_step = api(target, f"/cards/{maybe['number']}/steps", "POST",
                         {"step": {"content": "First task"}}, 201)
        api(target, f"/cards/{maybe['number']}/steps", "POST",
            {"step": {"content": "Second task"}}, 201)
        api(target, f"/cards/{maybe['number']}/steps/{first_step['id']}", "PATCH",
            {"step": {"completed": True}})
        blob = upload(target)
        api(target, f"/cards/{maybe['number']}.json", "PATCH",
            {"card": {"image": blob["signed_id"]}})
        api(target, f"/cards/{doing['number']}/triage", "POST", {"column_id": named["id"]}, 204)
        api(target, f"/cards/{not_now['number']}/not_now", "POST", expected=204)
        api(target, f"/cards/{done['number']}/closure", "POST", expected=204)
        publication = api(target, f"/boards/{board_id}/publication", "POST", expected=201)
        key = publication["public_url"].split("/public/boards/")[1]
        path = f"/public/boards/{key}"
        board_html, headers = anonymous(target, path)
        assert 'class="card-columns hide-scrollbar"' in board_html
        assert '<turbo-frame id="cards_container">' in board_html
        assert '<p>Welcome <strong>visitors</strong></p>' in board_html
        assert all(title not in board_html for title in (maybe["title"], doing["title"], done["title"]))
        assert board_name in meta(board_html, "og:title")
        assert "Welcome visitors" in meta(board_html, "og:description")
        assert meta(board_html, "og:url").endswith(path)
        with urllib.request.urlopen(meta(board_html, "og:image"), timeout=10) as image:
            assert image.status == 200 and image.headers.get_content_type() == "image/png"
        assert column(board_html, "not-now")[0] == "1"
        assert column(board_html, "maybe")[0] == "2"  # pinned Fizzy's fixed count
        assert column(board_html, "column_" + str(named["id"]))[0] == "1"
        assert column(board_html, "closed-cards")[0] == "1"
        for section in ("not-now", "maybe", "column_" + str(named["id"]), "closed-cards"):
            assert f"/{target['account']}/public/boards/{key}/columns/" in column(board_html, section)[1]
        assert "public" in headers.get("Cache-Control", "")

        card_path = path + f"/cards/{maybe['number']}"
        card_html, card_headers = anonymous(target, card_path)
        assert 'class="card-perma card-perma--public"' in card_html
        assert maybe["title"] in card_html and "<strong>bold</strong>" in card_html
        assert card_html.count('class="step"') == 2
        assert "First task" in card_html and "Second task" in card_html
        rendered_steps = re.findall(r'<li class="step".*?</li>', card_html, re.S)
        assert "checked" in rendered_steps[0] and "checked" not in rendered_steps[1]
        assert maybe["title"] in meta(card_html, "og:title")
        assert "Body bold" in meta(card_html, "og:description")
        assert meta(card_html, "og:url").endswith(card_path)
        image_url = meta(card_html, "og:image")
        assert "/rails/active_storage/blobs/redirect/" in image_url
        with urllib.request.urlopen(image_url, timeout=10) as image:
            assert image.status == 200 and image.read() == IMAGE
        assert "public" in card_headers.get("Cache-Control", "")
        api(target, f"/boards/{board_id}/publication", "DELETE", expected=204)
        anonymous(target, path, 404)
        anonymous(target, card_path, 404)
        return {"board_columns": 4, "card_steps": 2, "anonymous": True,
                "rich_board_description": True, "social_meta": True,
                "public_image": True, "unpublish_hides_pages": True}
    finally:
        api(target, f"/boards/{board_id}", "DELETE", expected=204)


def main():
    source = source_target()
    with tempfile.TemporaryDirectory() as temp:
        rust, process = rust_target(temp)
        try:
            suffix = str(time.time_ns())
            expected = run(source, suffix)
            actual = run(rust, suffix)
            assert actual == expected, (expected, actual)
            print("Public board and card HTML parity passed:", actual)
        finally:
            stop(process)


if __name__ == "__main__":
    main()
