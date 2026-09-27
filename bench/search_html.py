#!/usr/bin/env python3
"""Benchmark equivalent browser search fixtures after checking all result pages."""
import argparse
import hashlib
import http.cookiejar
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import urllib.parse
import urllib.request
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "tests"))
from auth import stop
from search_html_parity import api, rust_target, source_target, titles
from draft_workflow_parity import NoRedirect
import sustained_mixed as suite

from events_html import browser_cookie, html_response, run


def fixture(target, term, count, on_board):
    board = api(target, "/boards", "POST", {"board": {"name": "Search benchmark " + term}}, 201)
    board_id = board["id"]
    on_board(target, board_id)
    for i in range(count):
        card = api(target, f"/boards/{board_id}/cards", "POST", {
            "card": {"title": f"{term} card {i}", "description": f"Description {term} item {i}"}}, 201)
        if i % 5 == 0:
            api(target, f"/cards/{card['number']}/comments", "POST",
                {"comment": {"body": f"Comment about {term} item {i}"}}, 201)
    return board_id


def collect(target, term):
    all_titles = []
    pages = []
    for page in range(1, 5):
        path = "/search?" + urllib.parse.urlencode({"q": term, "page": page})
        status, _, body = html_response(target, path)
        assert status == 200, (target["name"], page, status)
        page_titles = titles(body.decode())
        pages.append({"page": page, "results": len(page_titles), "html_bytes": len(body)})
        all_titles.extend(re.sub(r"^# \d+ ", "", title) for title in page_titles)
    return pages, sorted(all_titles)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--cards", type=int, default=100)
    parser.add_argument("--seconds", type=float, default=2)
    parser.add_argument("--concurrency", default="1,8,32")
    parser.add_argument("--rounds", type=int, default=1, help="alternate target order across rounds")
    parser.add_argument("--fresh", action="store_true", help="start both apps on fresh SQLite databases")
    parser.add_argument("--cpus", type=int, default=8, help="shared CPU count for fresh servers and client")
    parser.add_argument("--workers", type=int, default=8, help="Fizzy Puma workers in fresh mode")
    args = parser.parse_args()
    assert args.cards >= 100 and args.cards % 5 == 0, "use at least 100 cards in multiples of five"
    assert args.rounds >= 1
    with tempfile.TemporaryDirectory() as temp:
        processes = []
        prior_cpus = os.sched_getaffinity(0)
        try:
            if args.fresh:
                cpus = sorted(prior_cpus)[:args.cpus]
                assert args.cpus > 0 and cpus and args.workers > 0
                targets_by_name = {}
                for name in ("fizzy", "rizzy"):
                    directory = Path(temp) / name
                    directory.mkdir()
                    env, credentials = (suite.prepare_source(directory, args.workers) if name == "fizzy"
                                        else suite.prepare_rust(directory))
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    process, url = suite.start_server(name, directory, env, cpus)
                    processes.append((process, suite.stop_server))
                    targets_by_name[name] = {
                        "name": name, "url": url, "account": str(credentials["account"]),
                        "token": credentials["token"], "email": "owner@example.com",
                        "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
                        "opener": urllib.request.build_opener(
                            NoRedirect(), urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar())),
                    }
                os.sched_setaffinity(0, cpus)
                source, rust = targets_by_name["fizzy"], targets_by_name["rizzy"]
            else:
                source = source_target()
                rust, process = rust_target(temp, release=True)
                processes.append((process, stop))
        except BaseException:
            os.sched_setaffinity(0, prior_cpus)
            for process, stopper in reversed(processes):
                stopper(process)
            raise
        targets = (source, rust)
        created = []
        try:
            term = "searchbench" + str(time.time_ns())
            for target in targets:
                fixture(target, term, args.cards, lambda target, board_id: created.append((target, board_id)))
                try:
                    target["cookie"] = browser_cookie(target)
                except Exception:
                    if args.fresh:
                        log = Path(temp) / target["name"] / (target["name"] + ".log")
                        print(log.read_text()[-2000:], file=sys.stderr)
                    raise
            pages = {}
            title_sets = {}
            for target in targets:
                pages[target["name"]], title_sets[target["name"]] = collect(target, term)
            expected = [min(max(args.cards + args.cards // 5 - offset, 0), limit)
                        for offset, limit in ((0, 15), (15, 30), (45, 50), (95, 100))]
            assert [page["results"] for page in pages["fizzy"]] == expected
            assert [page["results"] for page in pages["rizzy"]] == expected
            assert title_sets["fizzy"] == title_sets["rizzy"], {
                "source_only": sorted(set(title_sets["fizzy"]) - set(title_sets["rizzy"]))[:10],
                "rust_only": sorted(set(title_sets["rizzy"]) - set(title_sets["fizzy"]))[:10],
                "source_count": len(title_sets["fizzy"]),
                "rust_count": len(title_sets["rizzy"]),
            }
            results = {"fixture": {"cards": args.cards, "comments": args.cards // 5,
                                    "page_results": expected},
                       "samples": pages, "runs": {}}
            if args.fresh:
                results["environment"] = {
                    "fizzy_revision": subprocess.check_output(
                        ["git", "rev-parse", "HEAD"], cwd=suite.SOURCE, text=True).strip(),
                    "rizzy_sha256": hashlib.sha256(suite.RUST.read_bytes()).hexdigest(),
                    "shared_cpu_ids": cpus, "fizzy_workers": args.workers, "rizzy_processes": 1,
                    "rounds": args.rounds,
                }
            paths = {
                "first_page": "/search?" + urllib.parse.urlencode({"q": term, "page": 1}),
                "fourth_page": "/search?" + urllib.parse.urlencode({"q": term, "page": 4}),
            }
            for target in targets:
                for path in paths.values():
                    for _ in range(10):
                        status, _, _ = html_response(target, path)
                        assert status == 200
            for round_number in range(1, args.rounds + 1):
                ordered_targets = targets if round_number % 2 else tuple(reversed(targets))
                for label, path in paths.items():
                    for concurrency in map(int, args.concurrency.split(",")):
                        for target in ordered_targets:
                            result = run(target, path, concurrency, args.seconds)
                            result["round"] = round_number
                            results["runs"].setdefault(label, {}).setdefault(target["name"], []).append(result)
                            print(label, target["name"], result, file=sys.stderr, flush=True)
            print(json.dumps(results, indent=2))
        finally:
            for target, board_id in reversed(created):
                try:
                    api(target, f"/boards/{board_id}", "DELETE", expected=204)
                except Exception as error:
                    print("fixture cleanup failed", target["name"], error, file=sys.stderr)
            os.sched_setaffinity(0, prior_cpus)
            for process, stopper in reversed(processes):
                stopper(process)


if __name__ == "__main__":
    main()
