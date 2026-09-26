#!/usr/bin/env python3
"""Keep equal-time card ordering stable at the first-page boundary."""
import json
import tempfile
import urllib.parse

from auth import stop
from draft_workflow_parity import login, request
from search_html_parity import rust_target


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        try:
            login(target)
            account = target["account"]
            status, _, raw = request(target, f"/{account}/boards", "POST",
                                     {"board": {"name": "Card tie order"}}, bearer=True)
            assert status == 201, (status, raw[:200])
            board = json.loads(raw)
            titles = []
            for index in range(17):
                title = f"Same-time card {index:02d}"
                status, _, raw = request(target, f"/{account}/boards/{board['id']}/cards", "POST",
                                         {"card": {"title": title, "created_at": "2024-01-15T10:30:00Z"}}, bearer=True)
                assert status == 201, (status, raw[:200])
                titles.append(title)

            query = urllib.parse.urlencode({"board_ids[]": board["id"], "sorted_by": "oldest"})
            first = f"/{account}/cards.json?{query}"
            status, _, raw = request(target, first, bearer=True)
            assert status == 200, (status, raw[:200])
            assert [card["title"] for card in json.loads(raw)] == titles[:15]
            status, _, raw = request(target, first + "&page=2", bearer=True)
            assert status == 200, (status, raw[:200])
            assert [card["title"] for card in json.loads(raw)] == titles[15:]

            for path in (f"/{account}/cards?{query}",
                         f"/{account}/boards/{board['id']}?sorted_by=oldest"):
                status, _, raw = request(target, path, accept="text/html")
                assert status == 200, (path, status, raw[:200])
                body = raw.decode()
                positions = [body.index(title) for title in titles[:15]]
                assert positions == sorted(positions), path
                assert titles[15] not in body and titles[16] not in body

            status, headers, raw = request(target,
                f"/{account}/cards/previews.turbo_stream?{query}&target=cards_container",
                accept="text/vnd.turbo-stream.html")
            assert status == 200 and headers.get_content_type() == "text/vnd.turbo-stream.html"
            body = raw.decode()
            positions = [body.index(title) for title in titles[:15]]
            assert positions == sorted(positions)
            assert titles[15] not in body and titles[16] not in body
        finally:
            stop(process)
    print("Equal-time oldest cards keep ascending ID order across JSON, board HTML, and previews")


if __name__ == "__main__":
    main()
