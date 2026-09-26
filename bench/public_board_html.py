#!/usr/bin/env python3
"""Compare anonymous public-board and Maybe-column reads on equal fixtures."""
import argparse
import json
import re
import sys
import tempfile
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests"))
from auth import stop
from public_page_html_parity import column
from search_html_parity import api, rust_target, source_target

from events_html import html_response, run


def fixture(target, count, on_board):
    board = api(target, "/boards", "POST", {
        "board": {"name": "Public benchmark " + str(time.time_ns()),
                  "public_description": "<p>Public benchmark description</p>"}}, 201)
    board_id = board["id"]
    on_board(target, board_id)
    named = api(target, f"/boards/{board_id}/columns", "POST",
                {"column": {"name": "Doing"}}, 201)
    moved = count // 10
    for index in range(count):
        kind = ("not_now" if index < moved else
                "closed" if index < moved * 2 else
                "named" if index < moved * 3 else "maybe")
        card = api(target, f"/boards/{board_id}/cards", "POST", {
            "card": {"title": f"Public benchmark {kind} {index:04d}",
                     "description": "<p>Equal fixture content.</p>"}}, 201)
        number = card["number"]
        if kind == "not_now":
            api(target, f"/cards/{number}/not_now", "POST", expected=204)
        elif kind == "closed":
            api(target, f"/cards/{number}/closure", "POST", expected=204)
        elif kind == "named":
            api(target, f"/cards/{number}/triage", "POST",
                {"column_id": named["id"]}, 204)
    published = api(target, f"/boards/{board_id}/publication", "POST", expected=201)
    key = published["public_url"].split("/public/boards/")[1]
    return board_id, named["id"], key


def sample(target, board_id, column_id, key, count):
    target["cookie"] = ""  # The public routes must work without a session.
    path = f"/public/boards/{key}"
    status, headers, raw = html_response(target, path)
    assert status == 200 and "public" in headers.get("Cache-Control", "")
    document = raw.decode()
    assert "<p>Public benchmark description</p>" in document
    counts = {name: column(document, section)[0] for name, section in (
        ("maybe", "maybe"), ("not_now", "not-now"),
        ("named", "column_" + str(column_id)), ("closed", "closed-cards"))}
    expected = {"maybe": "2", "not_now": str(count // 10),
                "named": str(count // 10), "closed": str(count // 10)}
    # The source caps visible side-column counters at 99+.
    expected = {key: ("99+" if int(value) > 99 else value) for key, value in expected.items()}
    assert counts == expected, (target["name"], counts, expected)
    stream_path = path + "/columns/stream"
    status, _, stream = html_response(target, stream_path)
    assert status == 200
    titles = sorted(set(re.findall(r"Public benchmark maybe \d{4}", stream.decode())))
    assert len(titles) == 15, (target["name"], titles)
    return {"board_html_bytes": len(raw), "stream_html_bytes": len(stream),
            "column_counts": counts, "stream_titles": titles}, path, stream_path


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=2)
    parser.add_argument("--concurrency", default="1,8,32")
    args = parser.parse_args()
    assert args.cards >= 100 and args.cards % 10 == 0
    source = source_target()
    with tempfile.TemporaryDirectory() as temp:
        rust, process = rust_target(temp, release=True)
        targets = (source, rust)
        created = []
        try:
            samples, paths = {}, {}
            for target in targets:
                board, named, key = fixture(target, args.cards,
                                            lambda target, board: created.append((target, board)))
                samples[target["name"]], board_path, stream_path = sample(
                    target, board, named, key, args.cards)
                paths[target["name"]] = {"board": board_path, "stream": stream_path}
            assert samples["fizzy"]["column_counts"] == samples["rizzy"]["column_counts"]
            assert samples["fizzy"]["stream_titles"] == samples["rizzy"]["stream_titles"]
            for target in targets:
                for path in paths[target["name"]].values():
                    for _ in range(5):
                        status, _, _ = html_response(target, path)
                        assert status == 200
            results = {"fixture_cards": args.cards, "sample_parity": samples, "runs": {}}
            for label in ("board", "stream"):
                results["runs"][label] = {}
                for concurrency in map(int, args.concurrency.split(",")):
                    for target in targets:
                        result = run(target, paths[target["name"]][label], concurrency, args.seconds)
                        results["runs"][label].setdefault(target["name"], []).append(result)
                        print(label, target["name"], result, file=sys.stderr, flush=True)
            print(json.dumps(results, indent=2))
        finally:
            for target, board in reversed(created):
                try:
                    api(target, f"/boards/{board}", "DELETE", expected=204)
                except Exception as error:
                    print("fixture cleanup failed", target["name"], error, file=sys.stderr)
            stop(process)


if __name__ == "__main__":
    main()
