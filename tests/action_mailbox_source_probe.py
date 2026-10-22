#!/usr/bin/env python3
"""Record the pinned Fizzy revision's default production Action Mailbox responses."""

import os
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from sustained_mixed import prepare_source, start_server, stop_server  # noqa: E402
from card_image_parity import call  # noqa: E402


ROUTES = [
    ("POST", "/rails/action_mailbox/postmark/inbound_emails", 404),
    ("POST", "/rails/action_mailbox/relay/inbound_emails", 404),
    ("POST", "/rails/action_mailbox/sendgrid/inbound_emails", 404),
    ("GET", "/rails/action_mailbox/mandrill/inbound_emails", 404),
    ("POST", "/rails/action_mailbox/mandrill/inbound_emails", 404),
    ("POST", "/rails/action_mailbox/mailgun/inbound_emails/mime", 404),
    ("GET", "/rails/conductor/action_mailbox/inbound_emails", 403),
    ("GET", "/rails/conductor/action_mailbox/inbound_emails/new", 403),
    ("GET", "/rails/conductor/action_mailbox/inbound_emails/1", 403),
    ("POST", "/rails/conductor/action_mailbox/inbound_emails", 422),
    ("GET", "/rails/conductor/action_mailbox/inbound_emails/sources/new", 403),
    ("POST", "/rails/conductor/action_mailbox/inbound_emails/sources", 422),
    ("POST", "/rails/conductor/action_mailbox/1/reroute", 422),
    ("POST", "/rails/conductor/action_mailbox/1/incinerate", 422),
]


def main():
    with tempfile.TemporaryDirectory(prefix="fizzy-mailbox-probe-") as temporary:
        directory = Path(temporary)
        env, credentials = prepare_source(directory, 2)
        process, url = start_server("fizzy", directory, env,
                                    sorted(os.sched_getaffinity(0))[:2])
        target = {"url": url, "account": str(credentials["account"]),
                  "token": credentials["token"]}
        try:
            for method, path, expected in ROUTES:
                status, _, _ = call(target, path, method)
                assert status == expected, (method, path, expected, status)
        finally:
            stop_server(process)
    print("Pinned Fizzy production: 6 inactive ingresses and 8 gated conductor methods match")


if __name__ == "__main__":
    main()
