#!/usr/bin/env python3
"""Compare Home previews for local embeds, remote media, and card images."""

import json
import os
import sqlite3
import subprocess
import sys
import tempfile
from html.parser import HTMLParser
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "bench"))
from card_image_parity import IMAGE, call
from draft_workflow_parity import login, request
from embed_parity import upload
from events_html_parity import target
from sustained_mixed import (ROOT, prepare_rust, prepare_source, rust_env,
                             source_env, start_server, stop_server)


class PreviewFeatures(HTMLParser):
    def __init__(self):
        super().__init__()
        self.events = {}
        self.depth = 0
        self.action = None
        self.attachments_depth = None
        self.background_depth = None

    def handle_starttag(self, tag, attributes):
        attrs = dict(attributes)
        classes = attrs.get("class", "").split()
        if tag == "a" and "event" in classes:
            self.action = next((name[7:] for name in classes if name.startswith("event--")), None)
            if self.action:
                self.events[self.action] = {"images": [], "videos": [], "sources": [],
                                            "files": 0, "backgrounds": []}
                self.depth = 0
        if self.action:
            if "event_attachments" in classes:
                self.attachments_depth = self.depth + 1
            if "card__background" in classes:
                self.background_depth = self.depth + 1
            if tag == "img" and self.attachments_depth is not None:
                self.events[self.action]["images"].append(attrs.get("src", ""))
            if tag == "video" and self.attachments_depth is not None:
                self.events[self.action]["videos"].append(attrs.get("class", ""))
            if tag == "source" and self.attachments_depth is not None:
                self.events[self.action]["sources"].append(attrs.get("src", ""))
            if "attachment--file" in classes and self.attachments_depth is not None:
                self.events[self.action]["files"] += 1
            if tag == "img" and self.background_depth is not None:
                self.events[self.action]["backgrounds"].append(attrs.get("src", ""))
            if tag not in ("img", "input", "meta", "link", "br", "hr", "source"):
                self.depth += 1

    def handle_endtag(self, tag):
        if self.action and tag not in ("img", "input", "meta", "link", "br", "hr", "source"):
            self.depth -= 1
            if self.attachments_depth is not None and self.depth < self.attachments_depth:
                self.attachments_depth = None
            if self.background_depth is not None and self.depth < self.background_depth:
                self.background_depth = None
            if self.depth == 0:
                self.action = None


def response(target_item, path, method="GET", payload=None):
    status, _, raw = call(target_item, f"/{target_item['account']}{path}", method, payload)
    assert status in (200, 201), (target_item["name"], path, status, raw[:200])
    return json.loads(raw)


def check(item):
    board = response(item, "/boards", "POST", {"board": {"name": "Timeline media"}})
    image = upload(item)
    background = upload(item, filename="background.jpg")
    file_blob = upload(item, b"timeline notes\n", "notes.txt", "text/plain")
    image_tag = f'<action-text-attachment sgid="{image["attachable_sgid"]}"></action-text-attachment>'
    remote_image = ('<action-text-attachment url="https://example.com/sample.jpg" '
                    'content-type="image/jpeg" width="640" height="480"></action-text-attachment>')
    card = response(item, f"/boards/{board['id']}/cards", "POST", {
        "card": {"title": "Timeline media card", "image": background["signed_id"],
                 "description": f"<p>{image_tag}</p><p>{remote_image}</p>"}})
    file_tag = f'<action-text-attachment sgid="{file_blob["attachable_sgid"]}"></action-text-attachment>'
    remote_video = ('<action-text-attachment url="https://example.com/sample.mp4" '
                    'content-type="video/mp4" width="320" height="180"></action-text-attachment>')
    response(item, f"/cards/{card['number']}/comments", "POST", {
        "comment": {"body": f"<p>{file_tag}</p><p>{remote_video}</p>"}})
    login(item)
    status, _, body = request(item, f"/{item['account']}/events", accept="text/html")
    assert status == 200, (item["name"], status)
    parsed = PreviewFeatures()
    parsed.feed(body.decode())
    features = {name: parsed.events[name] for name in ("card_published", "comment_created")}
    assert len(features["card_published"]["backgrounds"]) == 1, (item["name"], features)
    assert len(features["comment_created"]["backgrounds"]) == 1, (item["name"], features)
    assert len(features["card_published"]["images"]) == 2, (item["name"], features)
    assert features["comment_created"]["files"] == 1, (item["name"], features)
    assert features["comment_created"]["sources"] == ["https://example.com/sample.mp4"], (
        item["name"], features)
    assert "https://example.com/sample.jpg" in features["card_published"]["images"], (
        item["name"], features)
    for feature in features.values():
        for path in feature["backgrounds"]:
            assert "/rails/active_storage/blobs/redirect/" in path
            status, headers, _ = call(item, path)
            assert status == 302, (item["name"], path, status)
            status, _, content = call(item, headers["Location"], bearer=False)
            assert status == 200 and content == IMAGE, (item["name"], path, status)
    local_image = next(path for path in features["card_published"]["images"]
                       if "/representations/redirect/" in path)
    status, headers, _ = call(item, local_image)
    assert status == 302, (item["name"], local_image, status)
    status, _, content = call(item, headers["Location"], bearer=False)
    assert status == 200 and content.startswith(b"\xff\xd8"), (item["name"], status)
    return {name: {"images": len(data["images"]), "videos": len(data["videos"]),
                   "files": data["files"], "backgrounds": len(data["backgrounds"])}
            for name, data in features.items()}


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-timeline-media-") as temporary:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(directory, 2,
                                                      source_env(directory, 2))
                    database = directory / "db.sqlite3"
                else:
                    environment = rust_env(directory)
                    environment["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    env, credentials = prepare_rust(directory, environment)
                    database = directory / "rizzy.sqlite3"
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                with sqlite3.connect(database) as db:
                    email = db.execute(
                        "SELECT i.email_address FROM accounts a JOIN users u "
                        "ON u.account_id=a.id JOIN identities i ON i.id=u.identity_id "
                        "WHERE a.external_account_id=? AND u.role='owner' LIMIT 1",
                        (credentials["account"],)).fetchone()[0]
                item = target(name, url, str(credentials["account"]),
                              credentials["token"], email, str(database))
                results[name] = check(item)
            assert results["fizzy"] == results["rizzy"], results
            print("PASS Home event media previews match Fizzy", results["rizzy"])
        finally:
            for process in processes:
                stop_server(process)


if __name__ == "__main__":
    main()
