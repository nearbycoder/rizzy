#!/usr/bin/env python3
"""Exercise account and board entropy identity and card timestamp effects."""
import sqlite3
import tempfile
import time

from auth import stop
from search_html_parity import api, rust_target


def record(target, kind, container_id):
    with sqlite3.connect(target["db"]) as db:
        return db.execute(
            "SELECT id,account_id,created_at,updated_at FROM entropy_records "
            "WHERE container_type=? AND container_id=?", (kind, container_id)).fetchone()


def card_times(target, card_id):
    with sqlite3.connect(target["db"]) as db:
        return db.execute("SELECT updated_at,last_active_at FROM cards WHERE id=?", (card_id,)).fetchone()


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        try:
            with sqlite3.connect(target["db"]) as db:
                account_id = db.execute("SELECT id FROM accounts WHERE external_account_id=?",
                                        (target["account"],)).fetchone()[0]
            account_first = record(target, "Account", account_id)
            assert account_first and account_first[2] == account_first[3]
            board = api(target, "/boards", "POST", {"board": {"name": "Entropy lifecycle"}}, 201)
            assert record(target, "Board", board["id"]) is None
            card = api(target, f"/boards/{board['id']}/cards", "POST", {
                "card": {"title": "Entropy touch"}}, 201)
            original_card = card_times(target, card["id"])

            time.sleep(.02)
            account = api(target, "/account/entropy", "PUT", {
                "entropy": {"auto_postpone_period_in_days": 7}})
            assert account["auto_postpone_period_in_days"] == 7
            account_changed = record(target, "Account", account_id)
            after_account = card_times(target, card["id"])
            assert account_changed[:3] == account_first[:3] and account_changed[3] > account_first[3]
            assert after_account[0] > original_card[0] and after_account[1] == original_card[1]
            assert api(target, f"/boards/{board['id']}")["auto_postpone_period_in_days"] == 7

            time.sleep(.02)
            api(target, "/account/entropy", "PUT", {
                "entropy": {"auto_postpone_period_in_days": 7}})
            assert record(target, "Account", account_id) == account_changed
            after_account_repeat = card_times(target, card["id"])
            assert after_account_repeat[0] > after_account[0]
            assert after_account_repeat[1] == after_account[1]

            time.sleep(.02)
            updated_board = api(target, f"/boards/{board['id']}/entropy", "PUT", {
                "board": {"auto_postpone_period_in_days": 90}})
            assert updated_board["auto_postpone_period_in_days"] == 90
            board_first = record(target, "Board", board["id"])
            after_board = card_times(target, card["id"])
            assert board_first and after_board[0] > after_account_repeat[0]
            assert after_board[1] == original_card[1]

            time.sleep(.02)
            api(target, f"/boards/{board['id']}/entropy", "PUT", {
                "board": {"auto_postpone_period_in_days": 90}})
            assert record(target, "Board", board["id"]) == board_first
            after_repeat = card_times(target, card["id"])
            assert after_repeat[0] > after_board[0] and after_repeat[1] == after_board[1]

            time.sleep(.02)
            api(target, "/account/entropy", "PUT", {
                "entropy": {"auto_postpone_period_in_days": 3}})
            assert record(target, "Board", board["id"]) == board_first
            assert api(target, f"/boards/{board['id']}")["auto_postpone_period_in_days"] == 90
            assert card_times(target, card["id"])[0] > after_repeat[0]

            api(target, f"/boards/{board['id']}", "DELETE", expected=204)
            assert record(target, "Board", board["id"]) is None
            assert record(target, "Account", account_id)[0] == account_first[0]
            print("Entropy lifecycle passed: account and board records, overrides, both repeat card touches, cleanup")
        finally:
            stop(process)


if __name__ == "__main__":
    main()
