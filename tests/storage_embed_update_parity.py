#!/usr/bin/env python3
"""Compare retained embeds across empty, null, and nonempty rich-text edits."""

import os
import time

from storage_ledger_parity import FILE, api, ledger, upload


def check(target):
    name = f"embed-update-{time.time_ns()}"
    board = api(target, "/boards", "POST", {"board": {"name": name}}, 201)
    board_names = {"board": name}
    try:
        card = api(target, f"/boards/{board['id']}/cards", "POST",
                   {"card": {"title": "Embed updates"}}, 201)
        first = upload(target, FILE, "first.txt", "text/plain")
        second = upload(target, FILE + b"two", "second.txt", "text/plain")
        board_blob = upload(target, FILE + b"board", "board.txt", "text/plain")
        snapshots = {}

        def card_edit(label, body):
            result = api(target, f"/cards/{card['number']}.json", "PATCH",
                         {"card": {"description": body}})
            read = api(target, f"/cards/{card['number']}.json")
            listed = api(target, f"/cards?board_ids%5B%5D={board['id']}")
            from_list = next(item for item in listed if item["id"] == card["id"])
            activities = api(target, f"/activities?board_ids%5B%5D={board['id']}")
            from_event = next(item["eventable"] for item in activities
                              if item["action"] == "card_published"
                              and item["eventable"]["id"] == card["id"])
            flags = (result["has_attachments"], read["has_attachments"],
                     from_list["has_attachments"], from_event["has_attachments"])
            assert len(set(flags)) == 1, (target["name"], label, flags)
            snapshots[label] = (flags, ledger(target, board_names))

        def board_edit(label, body):
            api(target, f"/boards/{board['id']}", "PATCH",
                {"board": {"public_description": body}})
            snapshots[label] = ledger(target, board_names)

        markup = lambda blob: (f'<p><action-text-attachment sgid="{blob["attachable_sgid"]}">'
                               '</action-text-attachment></p>')
        card_edit("card_attach", markup(first))
        card_edit("card_empty", "")
        card_edit("card_null", None)
        card_edit("card_plain", "<p>Plain text</p>")
        card_edit("card_replace", markup(second))
        board_edit("board_attach", markup(board_blob))
        board_edit("board_empty", "")
        board_edit("board_null", None)
        board_edit("board_plain", "<p>Plain text</p>")
        assert all(snapshots["card_attach"][0])
        assert all(snapshots["card_empty"][0]) and all(snapshots["card_null"][0])
        assert not any(snapshots["card_plain"][0]) and all(snapshots["card_replace"][0])
        assert snapshots["card_attach"][1] == snapshots["card_empty"][1]
        assert snapshots["card_empty"][1] == snapshots["card_null"][1]
        assert snapshots["board_attach"] == snapshots["board_empty"] == snapshots["board_null"]
        return snapshots
    finally:
        api(target, f"/boards/{board['id']}", "DELETE", expected=204)


def main():
    targets = [{"name": name.lower(), "url": os.environ[f"{name}_URL"],
                "account": os.environ[f"{name}_ACCOUNT"],
                "token": os.environ[f"{name}_TOKEN"],
                "db": os.environ[f"{name}_DB_PATH"]}
               for name in ("FIZZY", "RIZZY")]
    source, rust = (check(target) for target in targets)
    assert source == rust, (source, rust)
    print("Embed update parity passed across card, list, activity, and ledger")


if __name__ == "__main__":
    main()
