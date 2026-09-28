#!/usr/bin/env python3
"""Verify a MySQL card move rolls back when its event or comment cannot insert."""

import os
import subprocess
import sys
import tempfile
import uuid
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from card_image_parity import call  # noqa: E402
from card_workflow import expect  # noqa: E402
from sustained_mixed import prepare_rust, start_server, stop_server  # noqa: E402
from sustained_mixed_mysql import (  # noqa: E402
    create_databases, drop_databases, mysql_admin, mysql_query, rust_env,
)

CLIENT = os.environ.get("RIZZY_TEST_MYSQL_CLIENT", "/tmp/rizzy-mysql8/bin/mysql")
SOCKET = os.environ.get("RIZZY_TEST_MYSQL_SOCKET", "/tmp/rizzy-mysql8/mysql.sock")
PORT = int(os.environ.get("MYSQL_PORT", "33080"))


def main():
    if not Path(SOCKET).is_socket():
        raise SystemExit("Start the disposable MySQL server first")
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"],
                   cwd=ROOT, check=True)
    old_trust = mysql_admin(CLIENT, SOCKET, "SELECT @@GLOBAL.log_bin_trust_function_creators").strip()
    mysql_admin(CLIENT, SOCKET, "SET GLOBAL log_bin_trust_function_creators=1")
    database = "rzmoveatomic_" + uuid.uuid4().hex[:12]
    created = False
    server = None
    try:
        create_databases(CLIENT, SOCKET, [database])
        created = True
        with tempfile.TemporaryDirectory(prefix="rizzy-mysql-move-atomic-") as temp:
            directory = Path(temp)
            env, credentials = prepare_rust(
                directory, rust_env(directory, "127.0.0.1", PORT, database))
            server, url = start_server("rizzy", directory, env,
                                       sorted(os.sched_getaffinity(0))[:4])
            target = {"url": url, "account": str(credentials["account"]),
                      "token": credentials["token"]}
            source, _ = expect(target, "/boards", "POST",
                               {"board": {"name": "Atomic move source"}}, 201)
            destination, _ = expect(target, "/boards", "POST",
                                    {"board": {"name": "Atomic move destination"}}, 201)
            card, _ = expect(target, f"/boards/{source['id']}/cards", "POST",
                             {"card": {"title": "Atomic move card"}}, 201)
            card_id, number = card["id"], card["number"]
            path = f"/{target['account']}/cards/{number}/board"
            query = lambda statement: mysql_query(CLIENT, SOCKET, database, statement)

            def state():
                return query(
                    "SELECT (SELECT board_id FROM cards WHERE id='" + card_id + "'), "
                    "(SELECT COUNT(*) FROM events WHERE card_id='" + card_id +
                    "' AND action='card_board_changed'), "
                    "(SELECT COUNT(*) FROM comments WHERE card_id='" + card_id + "')")

            before = state()
            assert before == f"{source['id']}\t0\t0", before
            for table in ("events", "comments"):
                trigger = f"fail_board_move_{table}"
                query(f"CREATE TRIGGER {trigger} BEFORE INSERT ON {table} FOR EACH ROW "
                      "SIGNAL SQLSTATE '45000' SET MESSAGE_TEXT='forced move write failure'")
                try:
                    status, _, _ = call(target, path, "PATCH", {"board_id": destination["id"]})
                    assert status == 500, (table, status)
                    assert state() == before, (table, state(), before)
                finally:
                    query(f"DROP TRIGGER {trigger}")

            moved, _ = expect(target, f"/cards/{number}/board", "PATCH",
                              {"board_id": destination["id"]}, 200)
            assert moved["board"]["id"] == destination["id"], moved
            assert state() == f"{destination['id']}\t1\t1", state()
            same_time = query(
                "SELECT e.created_at=c.created_at FROM events e JOIN comments c "
                f"ON c.card_id=e.card_id WHERE e.card_id='{card_id}' "
                "AND e.action='card_board_changed'")
            assert same_time == "1", same_time
    finally:
        if server is not None:
            stop_server(server)
        if created:
            drop_databases(CLIENT, SOCKET, [database])
        mysql_admin(CLIENT, SOCKET,
                    f"SET GLOBAL log_bin_trust_function_creators={old_trust}")
    print("PASS MySQL board move is atomic across card, event, and comment")


if __name__ == "__main__":
    main()
