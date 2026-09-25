#!/usr/bin/env python3
"""Compare saved filter card results; seed the Rails filter through its model layer.

The Rails Turbo endpoint accepts browser sessions, not API bearer tokens. Its
create/destroy behavior is covered by the pinned Rails controller tests.
"""
import json
import os
import re
import subprocess
import time
import urllib.parse
import urllib.request
from pathlib import Path

from parity import request


ROOT = Path(__file__).resolve().parents[1]


def checked(target, path, method="GET", payload=None, expected=200):
    status, value = request(target, path, method, payload)
    assert status == expected, (path, status, value)
    return value


def create_rust_filter(target, board_id):
    body = urllib.parse.urlencode({"indexed_by": "closed", "board_ids[]": board_id}).encode()
    req = urllib.request.Request(
        f"{target['url']}/{target['account']}/filters", body, method="POST",
        headers={"Authorization": "Bearer " + target["token"], "Accept": "text/vnd.turbo-stream.html",
                 "Content-Type": "application/x-www-form-urlencoded"},
    )
    with urllib.request.urlopen(req, timeout=10) as response:
        assert response.status == 200
        html = response.read().decode()
    return re.search(r'/filters/([a-z0-9-]+)', html).group(1)


def delete_rust_filter(target, filter_id):
    req = urllib.request.Request(
        f"{target['url']}/{target['account']}/filters/{filter_id}", method="DELETE",
        headers={"Authorization": "Bearer " + target["token"], "Accept": "text/vnd.turbo-stream.html"},
    )
    with urllib.request.urlopen(req, timeout=10) as response:
        assert response.status == 200


def rails_runner(script, **values):
    ruby = Path("/home/nearby/.local/rizzy-ruby")
    env = {**os.environ,
           "PATH": str(ruby / "bin") + ":" + os.environ.get("PATH", ""),
           "LD_LIBRARY_PATH": str(ruby / "lib"),
           "BUNDLE_PATH": "/home/nearby/.local/rizzy-bundle",
           "RAILS_ENV": "production", "DISABLE_SSL": "true",
           "SECRET_KEY_BASE": "0123456789abcdef" * 4,
           **values}
    output = subprocess.run(["bin/rails", "runner", script], cwd=ROOT / "upstream", env=env,
                            capture_output=True, text=True, check=True)
    return output.stdout.strip()


def create_source_filter(target, board_name):
    script = '''token = Identity::AccessToken.find_by!(token: ENV.fetch("PARITY_TOKEN"))
      account = Account.find_by!(external_account_id: ENV.fetch("PARITY_ACCOUNT"))
      user = account.users.find_by!(identity: token.identity)
      board = account.boards.find_by!(name: ENV.fetch("PARITY_BOARD_NAME"))
      puts user.filters.remember(indexed_by: "closed", board_ids: [ board.id ]).id'''
    return rails_runner(script, PARITY_TOKEN=target["token"], PARITY_ACCOUNT=target["account"],
                        PARITY_BOARD_NAME=board_name)


def delete_source_filter(filter_id):
    rails_runner('Filter.find_by!(id: ENV.fetch("PARITY_FILTER_ID")).destroy!', PARITY_FILTER_ID=filter_id)


def main():
    targets = {name.lower(): {key.lower(): os.environ[f"{name}_{key}"] for key in ("URL", "ACCOUNT", "TOKEN")}
               for name in ("FIZZY", "RIZZY")}
    outputs = {}
    for name, target in targets.items():
        board_name = "Saved filter parity " + str(time.time_ns())
        board = checked(target, "/boards", "POST", {"board": {"name": board_name}}, 201)
        board_id = board["id"]
        filter_id = None
        try:
            cards = [checked(target, f"/boards/{board_id}/cards", "POST", {"card": {"title": title}}, 201)
                     for title in ("Open saved", "Closed saved")]
            checked(target, f"/cards/{cards[1]['number']}/closure", "POST", expected=204)
            filter_id = create_source_filter(target, board_name) if name == "fizzy" else create_rust_filter(target, board_id)
            output = checked(target, "/cards?filter_id=" + filter_id)
            outputs[name] = [card["title"] for card in output]
        finally:
            if filter_id:
                if name == "fizzy":
                    delete_source_filter(filter_id)
                else:
                    delete_rust_filter(target, filter_id)
            checked(target, "/boards/" + board_id, "DELETE", expected=204)
    assert outputs["fizzy"] == outputs["rizzy"] == ["Closed saved"], outputs
    print(json.dumps({"saved_filter_results": outputs}, indent=2))


if __name__ == "__main__":
    main()
