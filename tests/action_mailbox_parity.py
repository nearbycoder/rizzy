#!/usr/bin/env python3
"""Compare default production Action Mailbox routes on disposable servers."""

import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from action_mailbox_source_probe import ROUTES  # noqa: E402
from card_image_parity import call  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-mailbox-parity-") as temp:
        targets = {}
        processes = []
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temp) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                process, url = start_server(name, directory, env, sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                targets[name] = {"url": url, "token": credentials["token"]}

            cases = ROUTES + [
                ("POST", "/rails/action_mailbox/postmark/inbound_emails.json", 404),
                ("GET", "/rails/conductor/action_mailbox/inbound_emails.json", 403),
                ("POST", "/rails/conductor/action_mailbox/inbound_emails.json", 422),
            ]
            for bearer in (False, True):
                for accept in ("application/json", "text/html"):
                    for method, path, expected in cases:
                        responses = {}
                        for name, target in targets.items():
                            status, headers, body = call(
                                target, path, method, bearer=bearer, headers={"Accept": accept})
                            assert status == expected, (name, method, path, status, expected)
                            responses[name] = (status, headers.get("Content-Type"), body)
                        assert responses["fizzy"] == responses["rizzy"], (method, path, bearer, accept, responses)
        finally:
            for process in reversed(processes):
                stop_server(process)
    print("Action Mailbox production defaults matched on 14 methods, three JSON suffixes, two Accept values, and authenticated and anonymous requests")


if __name__ == "__main__":
    main()
