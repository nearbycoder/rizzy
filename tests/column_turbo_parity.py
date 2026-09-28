#!/usr/bin/env python3
"""Compare the named-column form and Turbo Stream lifecycle with Fizzy."""

import http.cookiejar
import os
import re
import subprocess
import sys
import tempfile
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from sustained_mixed import (prepare_rust, prepare_source, start_server,
                             stop_server)  # noqa: E402
from draft_workflow_parity import NoRedirect, login, request  # noqa: E402


def streams(body):
    result = []
    for opening in re.findall(rb"<turbo-stream\b([^>]*)>", body):
        attributes = {key.decode(): value.decode() for key, value in
                      re.findall(rb'([\w-]+)="([^"]*)"', opening)}
        result.append((attributes.get("action"), attributes.get("target")))
    return result


def check(name, directory=None, target_override=None):
    process = None
    if target_override is None:
        env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                            else prepare_rust(directory))
        if name == "rizzy":
            env["RIZZY_DEV_MAGIC_LINKS"] = "1"
        process, url = start_server(name, directory, env,
                                    sorted(os.sched_getaffinity(0))[:2])
        target = {"name": name, "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"], "email": "owner@example.com",
                  "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                  "opener": urllib.request.build_opener(
                      urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())}
    else:
        target = target_override
    try:
        login(target)
        board, _ = expect(target, "/boards", "POST",
                          {"board": {"name": "Column Turbo"}}, 201)
        board_path = f"/{target['account']}/boards/{board['id']}"
        first, _ = expect(target, f"/boards/{board['id']}/columns", "POST",
                          {"column": {"name": "First"}}, 201)
        second, _ = expect(target, f"/boards/{board['id']}/columns", "POST",
                           {"column": {"name": "Second"}}, 201)
        status, _, board_html = request(target, board_path, accept="text/html")
        assert status == 200, (name, status)
        csrf_match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', board_html)
        csrf = csrf_match.group(1).decode() if csrf_match else None
        form = lambda fields: urllib.parse.urlencode(fields).encode()
        status, headers, body = request(target, board_path + "/columns", "POST",
            form({"column[name]": "Third"}), accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200, (name, "create", status, body[:180])
        assert headers.get_content_type() == "text/vnd.turbo-stream.html", name
        created_streams = streams(body)
        assert ("before", "closed-cards") in created_streams, (name, created_streams)
        assert ("replace", f"column_{second['id']}") in created_streams, (name, created_streams)
        columns, _ = expect(target, f"/boards/{board['id']}/columns", "GET", None, 200)
        third = next(column for column in columns if column["name"] == "Third")
        position_path = f"/{target['account']}/columns/{second['id']}"
        status, headers, body = request(target, position_path + "/left_position", "POST",
            b"", accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200, (name, "move left", status, body[:180])
        assert headers.get_content_type() == "text/vnd.turbo-stream.html", name
        moved_left = streams(body)
        assert ("remove", f"column_{second['id']}") in moved_left, (name, moved_left)
        assert ("before", f"column_{first['id']}") in moved_left, (name, moved_left)
        assert ("replace", f"column_{first['id']}") in moved_left, (name, moved_left)
        status, headers, body = request(target, position_path + "/right_position", "POST",
            b"", accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200, (name, "move right", status, body[:180])
        moved_right = streams(body)
        assert ("remove", f"column_{second['id']}") in moved_right, (name, moved_right)
        assert ("after", f"column_{first['id']}") in moved_right, (name, moved_right)
        assert ("replace", f"column_{first['id']}") in moved_right, (name, moved_right)
        assert ("replace", f"column_{third['id']}") in moved_right, (name, moved_right)
        status, headers, body = request(target,
            f"/{target['account']}/columns/{first['id']}/left_position", "POST",
            b"", accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200 and not streams(body), (name, "left boundary", status, body[:180])
        status, headers, body = request(target,
            board_path + f"/columns/{second['id']}", "PUT",
            form({"column[name]": "Second renamed"}),
            accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert status == 200, (name, "update", status, body[:180])
        assert headers.get_content_type() == "text/vnd.turbo-stream.html", name
        updated_streams = streams(body)
        assert ("replace", f"column_{second['id']}") in updated_streams, (name, updated_streams)
        status, headers, body = request(target,
            board_path + f"/columns/{second['id']}", "DELETE", b"",
            accept="text/vnd.turbo-stream.html, text/html", csrf=csrf)
        assert status == 302 and not body, (name, "delete", status, body[:180])
        assert urllib.parse.urlparse(headers["Location"]).path == board_path, name
        remaining, _ = expect(target, f"/boards/{board['id']}/columns", "GET", None, 200)
        assert [column["id"] for column in remaining] == [first["id"], third["id"]]
        unsupported_status, _, _ = request(target,
            board_path + f"/columns/{third['id']}", "DELETE", b"",
            accept="text/vnd.turbo-stream.html", csrf=csrf)
        assert unsupported_status == 406, (name, unsupported_status)
        remaining_after_unsupported, _ = expect(
            target, f"/boards/{board['id']}/columns", "GET", None, 200)
        assert [column["id"] for column in remaining_after_unsupported] == [first["id"]]
        return {"create": created_streams, "move_left": moved_left,
                "move_right": moved_right, "update": updated_streams,
                "delete": status, "unsupported_delete": unsupported_status,
                "remaining": len(remaining_after_unsupported)}
    finally:
        if process is not None:
            stop_server(process)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-column-turbo-") as temporary:
        root = Path(temporary)
        results = {}
        for name in ("fizzy", "rizzy"):
            directory = root / name
            directory.mkdir()
            results[name] = check(name, directory)
        assert [item[0] for item in results["fizzy"]["create"]] == [
            item[0] for item in results["rizzy"]["create"]], results
    print("Column form Turbo actions, position swaps, and deletion redirect matched pinned Fizzy")


if __name__ == "__main__":
    main()
