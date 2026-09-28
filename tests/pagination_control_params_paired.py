#!/usr/bin/env python3
"""Check source and Rust next-page links ignore URL generation controls."""

import html.parser
import os
import subprocess
import sys
import tempfile
import urllib.parse
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from events_html import browser_cookie, html_response  # noqa: E402
from sustained_mixed import (fixture, prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)

CONTROL = {
    "script_name": "/rails/active_storage/blobs/proxy/SIGNED/x.html%23",
    "original_script_name": "/rails/active_storage/blobs/proxy/SIGNED/y.html%23",
    "host": "attacker.example",
    "protocol": "https",
    "port": "443",
    "subdomain": "attacker",
    "domain": "example.org",
    "tld_length": "2",
    "trailing_slash": "true",
    "only_path": "false",
    "relative_url_root": "/other",
    "anchor": "bad",
    "params": "bad",
    "_recall": "bad",
}


class Links(html.parser.HTMLParser):
    def __init__(self):
        super().__init__()
        self.items = []

    def handle_starttag(self, tag, attrs):
        if tag == "a":
            self.items.append(dict(attrs))


def check(target):
    target["cookie"] = browser_cookie(target)
    board_id, _, _ = fixture(target, 20)
    query = urllib.parse.urlencode({**CONTROL, "terms[]": "Fixture"})
    results = []
    for route in ("/cards", f"/boards/{board_id}"):
        status, _, body = html_response(target, f"{route}?{query}")
        assert status == 200, (target["name"], route, status, body[:100])
        links = Links()
        links.feed(body.decode())
        next_links = [item["href"] for item in links.items if item.get("href")
                      and urllib.parse.parse_qs(urllib.parse.urlsplit(item["href"]).query)
                      .get("page") == ["2"]]
        assert len(next_links) == 1, (target["name"], route, next_links)
        href = next_links[0]
        parts = urllib.parse.urlsplit(href)
        assert not parts.scheme and not parts.netloc, (target["name"], route, href)
        assert parts.path == f"/{target['account']}{route}", (target["name"], route, href)
        params = urllib.parse.parse_qs(parts.query)
        assert params.get("terms[]") == ["Fixture"], (target["name"], route, href)
        assert not (set(params) & set(CONTROL)), (target["name"], route, href)
        assert "active_storage" not in href and "attacker.example" not in href, href
        results.append((route.split("/")[1], sorted(params)))
    return results


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-pagination-controls-") as temporary:
        processes = []
        try:
            results = {}
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(directory, 2)
                else:
                    env = rust_env(directory)
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    env, credentials = prepare_rust(directory, env)
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url,
                          "account": str(credentials["account"]),
                          "token": credentials["token"], "email": "owner@example.com",
                          "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3"))}
                results[name] = check(target)
            assert results["fizzy"] == results["rizzy"], results
            print("PASS next-page paths and query params ignore URL generation controls")
        except Exception:
            for directory in Path(temporary).iterdir():
                log = directory / f"{directory.name}.log"
                if log.exists():
                    print(f"{directory.name} log: {log.read_text()[-1800:]}", file=sys.stderr)
            raise
        finally:
            for process in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
