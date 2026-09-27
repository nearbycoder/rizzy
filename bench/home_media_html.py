#!/usr/bin/env python3
"""Compare filtered Home HTML with equal card and media event fixtures."""

import argparse
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests"))

from compare import fixture, process_sample
from events_html import browser_cookie, html_response, run
from events_html_parity import target as browser_target
from embed_parity import upload
from sustained_mixed import ROOT, RUST, SOURCE, prepare_rust, prepare_source, start_server, stop_server
from timeline_event_media_parity import PreviewFeatures, response


class AllPreviewFeatures(PreviewFeatures):
    def __init__(self):
        super().__init__()
        self.all_events = []

    def handle_endtag(self, tag):
        action = self.action
        data = self.events.get(action) if action else None
        super().handle_endtag(tag)
        if action and self.action is None:
            self.all_events.append((action, data))


def revision(path):
    return subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=path, text=True).strip()


def add_media(target, board):
    image = upload(target)
    background = upload(target, filename="background.jpg")
    file_blob = upload(target, b"timeline notes\n", "notes.txt", "text/plain")
    image_tag = f'<action-text-attachment sgid="{image["attachable_sgid"]}"></action-text-attachment>'
    remote_image = ('<action-text-attachment url="https://example.com/sample.jpg" '
                    'content-type="image/jpeg" width="640" height="480"></action-text-attachment>')
    card = response(target, f"/boards/{board}/cards", "POST", {
        "card": {"title": "Timeline media card", "image": background["signed_id"],
                 "description": f"<p>{image_tag}</p><p>{remote_image}</p>"}})
    file_tag = f'<action-text-attachment sgid="{file_blob["attachable_sgid"]}"></action-text-attachment>'
    remote_video = ('<action-text-attachment url="https://example.com/sample.mp4" '
                    'content-type="video/mp4" width="320" height="180"></action-text-attachment>')
    response(target, f"/cards/{card['number']}/comments", "POST", {
        "comment": {"body": f"<p>{file_tag}</p><p>{remote_video}</p>"}})


def sample(target, path, cards):
    status, _, body = html_response(target, path)
    assert status == 200, (target["name"], status)
    parsed = AllPreviewFeatures()
    parsed.feed(body.decode())
    events = len(re.findall(rb'class="event event--', body))
    assert events == min(cards + 2, 100), (target["name"], events)
    assert body.count(b'class="events__column"') == 3
    assert b"Timeline media card" in body and b"Benchmark card 0" in body
    published = next(data for action, data in parsed.all_events
                     if action == "card_published" and len(data["images"]) == 2)
    commented = next(data for action, data in parsed.all_events
                     if action == "comment_created" and data["files"] == 1)
    media = {"published_images": len(published["images"]),
             "published_backgrounds": len(published["backgrounds"]),
             "comment_files": commented["files"],
             "comment_videos": len(commented["sources"]),
             "comment_backgrounds": len(commented["backgrounds"])}
    assert media == {"published_images": 2, "published_backgrounds": 1,
                     "comment_files": 1, "comment_videos": 1,
                     "comment_backgrounds": 1}, (target["name"], media)
    return {"events": events, "columns": 3, "media": media, "html_bytes": len(body)}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=98)
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--concurrency", default="8,32")
    parser.add_argument("--rounds", type=int, default=2)
    args = parser.parse_args()
    assert 1 <= args.cards <= 98
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    cpus = sorted(os.sched_getaffinity(0))
    assert len(cpus) >= 8
    server_cpus, client_cpus = cpus[:4], cpus[4:12]
    results = {"source_revision": revision(SOURCE), "rizzy_revision": revision(ROOT),
               "rizzy_binary_sha256": hashlib.sha256(Path(RUST).read_bytes()).hexdigest(),
               "server_cpus": server_cpus, "client_cpus": client_cpus,
               "fizzy_workers": 4, "rizzy_processes": 1, "plain_cards": args.cards,
               "seconds_per_run": args.seconds, "samples": {}, "runs": []}
    original_affinity = os.sched_getaffinity(0)
    with tempfile.TemporaryDirectory(prefix="rizzy-home-media-bench-") as temporary:
        processes, targets = [], {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 4) if name == "fizzy"
                                    else prepare_rust(directory))
                if name == "rizzy":
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                process, url = start_server(name, directory, env, server_cpus)
                processes.append(process)
                target = browser_target(name, url, str(credentials["account"]),
                                        credentials["token"], "owner@example.com",
                                        str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")))
                board, _ = fixture(target, args.cards, board_name="Home media benchmark")
                add_media(target, board)
                target["cookie"] = browser_cookie(target)
                path = "/events?board_ids%5B%5D=" + board
                target["path"] = path
                results["samples"][name] = sample(target, path, args.cards)
                for _ in range(5):
                    assert html_response(target, path)[0] == 200
                target["pid"] = process.pid
                targets[name] = target
            assert results["samples"]["fizzy"]["events"] == results["samples"]["rizzy"]["events"]
            assert results["samples"]["fizzy"]["media"] == results["samples"]["rizzy"]["media"]
            os.sched_setaffinity(0, client_cpus)
            for round_index in range(args.rounds):
                names = ("fizzy", "rizzy") if round_index % 2 == 0 else ("rizzy", "fizzy")
                for concurrency in map(int, args.concurrency.split(",")):
                    for name in names:
                        target = targets[name]
                        before = process_sample(target["pid"])
                        result = run(target, target["path"], concurrency, args.seconds)
                        after = process_sample(target["pid"])
                        result["server_cpu_core_average"] = round(
                            (after["cpu_seconds"] - before["cpu_seconds"]) / result["seconds"], 2)
                        result["server_pss_mib_end"] = round(after["pss_mib"], 1)
                        assert not result["errors"], (name, result)
                        results["runs"].append({"round": round_index + 1, "target": name, **result})
                        print(f"round {round_index + 1} {name} c={concurrency}: "
                              f"{result['rps']:.1f} req/s, p95 {result['p95_ms']:.1f} ms",
                              file=sys.stderr, flush=True)
        finally:
            os.sched_setaffinity(0, original_affinity)
            for process in processes:
                stop_server(process)
    print(json.dumps(results, indent=2))


if __name__ == "__main__":
    main()
