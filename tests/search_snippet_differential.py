#!/usr/bin/env python3
"""Compare SQLite search excerpts, number lookup, and tied pagination."""
import http.cookiejar
import os
import random
import re
import sys
import tempfile
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
import sustained_mixed as suite  # noqa: E402
from card_workflow import expect  # noqa: E402
from draft_workflow_parity import NoRedirect, login  # noqa: E402
from search_html_parity import api, card_excerpts, html, titles  # noqa: E402

CARDS = [
    "alpha beta", "gamma delta", "running fast", "run slowly", "cafe menu",
    "café music", "東京 plan", "foo-bar alpha", "123 beta", "abc_def gamma",
    "and or not", "will do", "do not wait", "alpha 123 beta",
    "beta gamma alpha", "quoted word", "star power", "plus minus",
    "tree trees", "layout broken", "test card", "number 999", "road-map",
    "phrase one two", "pipe marker",
]
TERMS = [
    "alpha", "beta", "gamma", "run", "running", "cafe", "café", "東京",
    "foo", "bar", "123", "abc_def", "and", "or", "not", "tree",
    "trees", "layout", "broken", "road", "map", "phrase", "one",
    "two", "999", "word", "context",
]


def queries():
    values = list(TERMS)
    for term in TERMS:
        values.extend((f'"{term}"', term + "*", term + "|beta", term + '"beta'))
    randomizer = random.Random(2901)
    for _ in range(180):
        parts = randomizer.choices(TERMS, k=randomizer.randint(1, 3))
        values.append(randomizer.choice((" ", "|", "+", "-", "(", ")", '"')).join(parts))
    return list(dict.fromkeys(values))


def setup(name, directory, cpus):
    env, credentials = (suite.prepare_source(directory, 1) if name == "fizzy"
                        else suite.prepare_rust(directory))
    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
    process, url = suite.start_server(name, directory, env, cpus)
    try:
        target = {
            "name": name, "url": url, "account": str(credentials["account"]),
            "token": credentials["token"],
            "db": str(directory / ("db.sqlite3" if name == "fizzy" else "rizzy.sqlite3")),
            "email": "owner@example.com",
            "opener": urllib.request.build_opener(
                urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect()),
        }
        login(target)
        board, _ = expect(target, "/boards", "POST", {"board": {"name": "Search snippets"}}, 201)
        for index, title in enumerate(CARDS):
            description = "Context before " + title + " context after "
            if index % 3 == 0:
                description += "word " * 20
            expect(target, f"/boards/{board['id']}/cards", "POST",
                   {"card": {"title": title, "description": description}}, 201)
        for index in range(110):
            expect(target, f"/boards/{board['id']}/cards", "POST", {"card": {
                "title": f"tieproof item {index:03d}",
                "created_at": "2026-01-01T00:00:00.000Z",
            }}, 201)
        return process, target
    except Exception:
        suite.stop_server(process)
        raise


def main():
    if binary := os.environ.get("RIZZY_TEST_BINARY"):
        suite.RUST = Path(binary)
    with tempfile.TemporaryDirectory(prefix="rizzy-search-snippets-") as temporary:
        targets = {}
        processes = []
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                process, target = setup(name, directory, sorted(os.sched_getaffinity(0))[:2])
                targets[name] = target
                processes.append(process)
            differences = []
            for term in queries():
                rendered = {}
                for name, target in targets.items():
                    page = html(target, term)
                    rendered[name] = (titles(page), card_excerpts(page))
                if rendered["fizzy"] != rendered["rizzy"]:
                    differences.append((term, rendered))
            assert not differences, differences[:8]
            for name, target in targets.items():
                for term, number in (("123*", 123), ("123|beta", 123),
                                     ('123"beta', 123), ("+123", 123), ("1_2", 12)):
                    path = "/search?" + urllib.parse.urlencode({"q": term})
                    hits = api(target, path)
                    assert len(hits) == 1 and hits[0]["number"] == number, (name, term, hits)
                    assert f"/cards/{number}" in html(target, term), (name, term)
            page_counts = {}
            for name, target in targets.items():
                all_titles = []
                for page in range(1, 5):
                    page_titles = titles(html(target, "tieproof", page))
                    all_titles.extend(re.sub(r"^# \d+ ", "", title) for title in page_titles)
                assert len(all_titles) == len(set(all_titles)) == 110, (name, len(all_titles))
                page_counts[name] = sorted(all_titles)
            assert page_counts["fizzy"] == page_counts["rizzy"]
            print(f"Search titles and snippets matched on {len(queries())} signed-in queries; numeric lookups and 110 equal-time hits also matched")
        finally:
            for process in reversed(processes):
                suite.stop_server(process)


if __name__ == "__main__":
    main()
