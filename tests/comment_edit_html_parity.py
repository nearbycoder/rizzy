#!/usr/bin/env python3
"""Compare comment edit pages and JSON updates against a running Fizzy."""
import http.cookiejar
import datetime
import json
import os
import re
import sqlite3
import time
import urllib.request

from draft_workflow_parity import NoRedirect, login, request


def target(name):
    return {"name":name.lower(), "url":os.environ[f"{name}_URL"],
            "account":os.environ[f"{name}_ACCOUNT"], "token":os.environ[f"{name}_TOKEN"],
            "email":os.environ[f"{name}_EMAIL"], "db":os.environ[f"{name}_DB"],
            "opener":urllib.request.build_opener(
                urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}


def check(target):
    login(target)
    account = target["account"]
    boards = []
    try:
        status, _, raw = request(target, f"/{account}/boards", "POST",
                                 {"board":{"name":f"Comment edit parity {time.time_ns()}"}}, bearer=True)
        assert status == 201, (target["name"], "board", status)
        board = json.loads(raw)
        boards.append(board["id"])
        status, _, raw = request(target, f"/{account}/boards/{board['id']}/cards", "POST",
                                 {"card":{"title":"Edit my comment"}}, bearer=True)
        assert status == 201, (target["name"], "card", status)
        card = json.loads(raw)
        number = card["number"]
        status, _, _ = request(target, f"/{account}/cards/{number}/watch", "DELETE", bearer=True)
        assert status == 204, (target["name"], "unwatch", status)
        with sqlite3.connect(target["db"]) as db:
            watch_rows = db.execute("SELECT watching FROM watches WHERE card_id=(SELECT id FROM cards WHERE number=?)", (number,)).fetchall()
        assert watch_rows == [(0,)], (target["name"], "before comment", watch_rows)
        status, _, first_card_html = request(target, f"/{account}/cards/{number}", accept="text/html")
        assert status == 200 and "Next, add some notes, context, pictures, or video about this…".encode() in first_card_html
        status, _, raw = request(target, f"/{account}/cards/{number}/comments", "POST",
                                 {"comment":{"body":"Original comment"}}, bearer=True)
        assert status == 201, (target["name"], "comment", status)
        comment_id = json.loads(raw)["id"]
        with sqlite3.connect(target["db"]) as db:
            watch_rows = db.execute("SELECT watching FROM watches WHERE card_id=(SELECT id FROM cards WHERE number=?)", (number,)).fetchall()
        assert watch_rows == [(1,)], (target["name"], "comment rewatch", watch_rows)
        path = f"/{account}/cards/{number}/comments/{comment_id}"
        index_path = f"/{account}/cards/{number}/comments"
        for method in ("POST", "DELETE"):
            status, _, _ = request(target, f"/{account}/cards/{number}/closure", method, b"", bearer=True)
            assert status == 204, (target["name"], "closure", method, status)
        status, _, raw = request(target, index_path, bearer=True)
        assert status == 200 and any(comment["id"] == comment_id for comment in json.loads(raw))
        assert request(target, index_path, accept="text/html")[0] == 406
        assert request(target, path, accept="text/vnd.turbo-stream.html")[0] == 406
        status, _, body = request(target, path + "/edit", accept="text/html")
        assert status == 200, (target["name"], "edit", status)
        csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', body)
        csrf = csrf_match.group(1).decode() if csrf_match else None
        new_target = f"new_comment_card_{card['id']}".encode()
        status, _, card_html = request(target, f"/{account}/cards/{number}", accept="text/html")
        assert status == 200 and new_target in card_html
        assert b'Type your comment\xe2\x80\xa6' in card_html
        assert card_html.count(b'class="comment-by-system"') == 2, target["name"]
        assert b'Full history' in card_html and b'Reopened' in card_html, target["name"]
        for marker in (f'messages_card_{card["id"]}'.encode(),
                       f'comment_watchers_card_{card["id"]}'.encode(),
                       b'aria-label="Messages"', b'comment__avatar',
                       b'comment__content', b'comment__author', b'comment__body',
                       b'data-local-time-target="agoorweekday"', b'data-local-time-target="time"'):
            assert marker in card_html, (target["name"], "message layout", marker)
        assert re.search(rb'<time datetime="\d+" data-local-time-target="agoorweekday"', card_html), target["name"]
        status, headers, created_stream = request(target, f"/{account}/cards/{number}/comments",
                                                   "POST", {"comment":{"body":"Turbo-created comment"}},
                                                   accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html"
        assert b'action="before"' in created_stream and b'action="update"' in created_stream
        assert created_stream.count(new_target) >= 2 and b"Turbo-created comment" in created_stream
        status, headers, created_stream = request(target, f"/{account}/cards/{number}/comments",
                                                   "POST", b"comment%5Bbody%5D=Form+Turbo+comment",
                                                   accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html"
        assert b'action="before"' in created_stream and b'action="update"' in created_stream
        assert b"Form Turbo comment" in created_stream
        backdated = "2024-01-15T10:30:00Z"
        status, _, raw = request(target, f"/{account}/cards/{number}/comments", "POST",
                                 {"comment":{"body":"Backdated comment","created_at":backdated}}, bearer=True)
        assert status == 201
        saved_time = json.loads(raw)["created_at"].replace('Z', '+00:00')
        assert datetime.datetime.fromisoformat(saved_time) == datetime.datetime.fromisoformat(backdated.replace('Z', '+00:00'))
        status, _, raw = request(target, index_path, "POST", {"body":"Flat JSON comment"}, bearer=True)
        assert status == 201 and json.loads(raw)["body"]["plain_text"] == "Flat JSON comment"
        flat_id = json.loads(raw)["id"]
        status, _, raw = request(target, f"{index_path}/{flat_id}", "PUT",
                                 {"body":"Flat JSON update"}, bearer=True)
        assert status == 200 and json.loads(raw)["body"]["plain_text"] == "Flat JSON update"
        for marker in (b"turbo-frame", b"Original comment", b"comment[body]", b"Save", b"Cancel", b"Delete", b"lexxy-editor", b"comment__avatar"):
            assert marker in body, (target["name"], marker)
        status, _, _ = request(target, path + "/edit.json", bearer=True)
        assert status == 406, (target["name"], "JSON edit", status)
        status, _, raw = request(target, path, "PATCH",
                                 {"comment":{"body":"Updated comment"}}, bearer=True)
        assert status == 200 and "Updated comment" in json.loads(raw)["body"]["plain_text"], (target["name"], "update", status)
        status, _, raw = request(target, path, bearer=True)
        assert status == 200 and "Updated comment" in json.loads(raw)["body"]["plain_text"]
        frame_id = f"container_comment_{comment_id}".encode()
        status, _, frame = request(target, path, accept="text/html")
        assert status == 200 and frame_id in frame and b"Updated comment" in frame
        status, _, body = request(target, f"/{account}/cards/{number}", accept="text/html")
        assert status == 200 and b"Updated comment" in body and frame_id in body
        status, headers, stream = request(target, path, "PATCH",
                                          {"comment":{"body":"Updated in frame"}},
                                          accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html"
        assert b'action="replace"' in stream and frame_id in stream and b"Updated in frame" in stream
        status, _, raw = request(target, path, bearer=True)
        assert status == 200 and "Updated in frame" in json.loads(raw)["body"]["plain_text"]
        status, headers, stream = request(target, path, "POST",
                                          b"_method=patch&comment%5Bbody%5D=Updated+by+Turbo+form",
                                          accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html"
        assert b'action="replace"' in stream and b"Updated by Turbo form" in stream
        status, _, raw = request(target, path, bearer=True)
        assert status == 200 and "Updated by Turbo form" in json.loads(raw)["body"]["plain_text"]
        status, _, _ = request(target, path, "PATCH",
                               {"comment":{"body":"Saved despite HTML 406"}},
                               accept="text/html", csrf=csrf)
        assert status == 406, (target["name"], "HTML update", status)
        status, _, raw = request(target, path, bearer=True)
        assert status == 200 and json.loads(raw)["body"]["plain_text"] == "Saved despite HTML 406"
        status, _, _ = request(target, path, "POST",
                               b"_method=patch&comment%5Bbody%5D=Saved+by+HTML+form",
                               accept="text/html", csrf=csrf)
        assert status == 406, (target["name"], "HTML form update", status)
        status, _, raw = request(target, path, bearer=True)
        assert status == 200 and json.loads(raw)["body"]["plain_text"] == "Saved by HTML form"
        status, _, _ = request(target, index_path, "POST",
                               {"comment":{"body":"Created despite HTML 406"}},
                               accept="text/html", csrf=csrf)
        assert status == 406, (target["name"], "HTML create", status)
        status, _, raw = request(target, index_path, bearer=True)
        assert status == 200 and any(comment["body"]["plain_text"] == "Created despite HTML 406"
                                     for comment in json.loads(raw)), target["name"]
        status, _, _ = request(target, path, "DELETE", accept="text/html", csrf=csrf)
        assert status == 406, (target["name"], "unsupported HTML delete", status)
        assert request(target, path, bearer=True)[0] == 404
        status, _, raw = request(target, f"/{account}/cards/{number}/comments", "POST",
                                 {"comment":{"body":"Remove as Turbo"}}, bearer=True)
        assert status == 201
        turbo_comment = json.loads(raw)["id"]
        turbo_path = f"/{account}/cards/{number}/comments/{turbo_comment}"
        status, headers, stream = request(target, turbo_path, "DELETE",
                                          accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html"
        assert b'action="remove"' in stream and f"container_comment_{turbo_comment}".encode() in stream
        assert request(target, turbo_path, bearer=True)[0] == 404
        status, _, raw = request(target, f"/{account}/cards/{number}/comments", "POST",
                                 {"comment":{"body":"Form comment"}}, bearer=True)
        assert status == 201
        form_comment = json.loads(raw)["id"]
        form_path = f"/{account}/cards/{number}/comments/{form_comment}"
        status, _, _ = request(target, form_path, "POST", b"_method=delete",
                               accept="text/html", csrf=csrf)
        assert status == 406, (target["name"], "HTML form delete", status)
        assert request(target, form_path, bearer=True)[0] == 404
        return {"edit":200,"json_edit":406,"json_update":200,"browser_card":200,
                "json_index":200,"html_index":406,"turbo_show":406,"flat_json":200,
                "turbo_create":200,"turbo_form_create":200,"backdated_create":201,
                "html_show":200,"turbo_update":200,"turbo_form":200,
                "turbo_destroy":200,"html_delete":406,"html_update":406,
                "html_create":406,"html_form_update":406,"html_form_delete":406}
    finally:
        for board_id in boards:
            status, _, _ = request(target, f"/{account}/boards/{board_id}", "DELETE", bearer=True)
            assert status == 204, (target["name"], "cleanup", status)


def main():
    with sqlite3.connect(os.environ.get("FIZZY_DB", "upstream/storage/production.sqlite3")) as db:
        email = db.execute("SELECT i.email_address FROM accounts a JOIN users u ON u.account_id=a.id "
                           "JOIN identities i ON i.id=u.identity_id WHERE a.external_account_id=? "
                           "AND u.role='owner' LIMIT 1", (int(os.environ["FIZZY_ACCOUNT"]),)).fetchone()[0]
    os.environ["FIZZY_EMAIL"] = email
    source, rust = check(target("FIZZY")), check(target("RIZZY"))
    assert source == rust
    print("Comment parity matched: supported formats, flat JSON params, Turbo create/edit/delete, HTML frames, JSON timestamps, and HTML delete side effect")


if __name__ == "__main__":
    main()
