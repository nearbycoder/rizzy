#!/usr/bin/env python3
"""Compare fresh Fizzy and Rizzy named-column HTML reads on equal fixtures."""

import argparse
import hashlib
import http.cookiejar
import json
import os
import re
import sqlite3
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

from compare import request
from events_html import html_response, run
from sustained_mixed import (ROOT, RUST, SOURCE, prepare_rust,
                             prepare_source, start_server, stop_server)


def revision(directory):
    return subprocess.check_output(["git", "rev-parse", "HEAD"],
                                   cwd=directory, text=True).strip()


def login_cookie(target):
    jar = http.cookiejar.CookieJar()
    opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar))
    headers = {"Accept": "application/json", "Content-Type": "application/json"}
    email = target["email"]
    request = urllib.request.Request(target["url"] + "/session",
        json.dumps({"email_address": email}).encode(), headers=headers)
    with opener.open(request, timeout=20) as response:
        code = response.headers.get("X-Magic-Link-Code")
        assert response.status == 201, (target["name"], response.status)
    if not code:
        with sqlite3.connect(target["db"]) as db:
            code = db.execute(
                "SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id "
                "WHERE i.email_address=? ORDER BY m.created_at DESC LIMIT 1", (email,)
            ).fetchone()[0]
    request = urllib.request.Request(target["url"] + "/session/magic_link",
        json.dumps({"code": code}).encode(), headers=headers)
    with opener.open(request, timeout=20) as response:
        assert response.status == 200, (target["name"], response.status)
    test = urllib.request.Request(target["url"] + f"/{target['account']}/boards")
    jar.add_cookie_header(test)
    cookie = test.get_header("Cookie")
    assert cookie, target["name"]
    return cookie


def prepare(name, directory, cpus):
    env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                        else prepare_rust(directory))
    if name == "rizzy":
        env["RIZZY_DEV_MAGIC_LINKS"] = "1"
    process, url = start_server(name, directory, env, cpus)
    target = {"name": name, "url": url, "account": str(credentials["account"]),
              "token": credentials["token"], "email": "owner@example.com",
              "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3"))}
    try:
        target["cookie"] = login_cookie(target)
        board = request(target, "/boards", "POST",
                        {"board": {"name": "Named column benchmark"}})
        column = request(target, f"/boards/{board['id']}/columns", "POST",
                         {"column": {"name": "In progress"}})
        for index in range(16):
            card = request(target, f"/boards/{board['id']}/cards", "POST",
                           {"card": {"title": f"Benchmark card {index:02d}"}})
            request(target, f"/cards/{card['number']}/triage", "POST",
                    {"column_id": column["id"]})
        path = f"/boards/{board['id']}/columns/{column['id']}"
        status, headers, body = html_response(target, path)
        assert status == 200 and headers.get("ETag"), (name, status)
        assert len(re.findall(rb'id="article_card_[^"]+"', body)) == 15, name
        assert b"cards--grid" in body and b"pagination-link-2" in body, name
        assert b"Benchmark card 15" in body, name
        second_status, _, second = html_response(target, path + "?page=2")
        assert second_status == 200, (name, second_status)
        assert len(re.findall(rb'id="article_card_[^"]+"', second)) == 1, name
        for _ in range(5):
            assert html_response(target, path)[0] == 200
        return process, target, path, {"first_page_cards": 15,
                                       "second_page_cards": 1,
                                       "first_page_bytes": len(body),
                                       "second_page_bytes": len(second)}
    except BaseException:
        stop_server(process)
        raise


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--seconds", type=float, default=5)
    parser.add_argument("--concurrency", default="1,8,32")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    levels = [int(value) for value in args.concurrency.split(",")]
    assert args.seconds > 0 and all(value > 0 for value in levels)
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    cpus = sorted(os.sched_getaffinity(0))[:8]
    assert len(cpus) == 8, "Benchmark needs eight available CPUs"
    result = {"source_revision": revision(SOURCE), "rizzy_revision": revision(ROOT),
              "rizzy_binary_sha256": hashlib.sha256(RUST.read_bytes()).hexdigest(),
              "server_cpus": cpus, "source_workers": 2,
              "fixture_cards": 16, "seconds_per_phase": args.seconds,
              "sample": {}, "phases": []}
    with tempfile.TemporaryDirectory(prefix="rizzy-named-column-bench-") as temporary:
        root = Path(temporary)
        servers = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = root / name
                directory.mkdir()
                servers[name] = prepare(name, directory, cpus)
                result["sample"][name] = servers[name][3]
            for concurrency in levels:
                for round_number, order in enumerate((("fizzy", "rizzy"),
                                                      ("rizzy", "fizzy")), 1):
                    for name in order:
                        _, target, path, _ = servers[name]
                        phase = run(target, path, concurrency, args.seconds)
                        assert not phase["errors"] and phase["requests"] > 0, (name, phase)
                        result["phases"].append({"round": round_number,
                                                 "name": name, **phase})
                        print(f"{name} c={concurrency} round={round_number}: "
                              f"{phase['rps']} req/s, p95={phase['p95_ms']} ms",
                              file=sys.stderr, flush=True)
        finally:
            for process, _, _, _ in servers.values():
                stop_server(process)
    for concurrency in levels:
        for round_number in (1, 2):
            pair = {phase["name"]: phase for phase in result["phases"]
                    if phase["concurrency"] == concurrency
                    and phase["round"] == round_number}
            result.setdefault("comparisons", []).append({
                "concurrency": concurrency, "round": round_number,
                "rps_ratio_rizzy_over_fizzy": round(
                    pair["rizzy"]["rps"] / pair["fizzy"]["rps"], 3),
                "p95_ratio_fizzy_over_rizzy": round(
                    pair["fizzy"]["p95_ms"] / pair["rizzy"]["p95_ms"], 3),
            })
    serialized = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_text(serialized)
    print(serialized, end="")


if __name__ == "__main__":
    main()
