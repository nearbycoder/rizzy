#!/usr/bin/env python3
"""Check mixed same-board card writes through four MySQL-backed processes.

Set RIZZY_TEST_MYSQL_URL to a MySQL database dedicated to tests. Each run adds
one account and 400 cards. This checks correctness, not a throughput target.
Set RIZZY_TEST_MYSQL_CLIENT to a MySQL CLI to verify persisted watch states.
"""

import concurrent.futures
import json
import os
import re
import signal
import socket
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
import uuid
from pathlib import Path
from urllib.parse import unquote, urlparse


ROOT = Path(__file__).resolve().parents[1]
BINARY = ROOT / "target/debug/rizzy"


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def call(base, account, token, path, method="GET", body=None):
    request = urllib.request.Request(
        base + f"/{account}" + path,
        json.dumps(body).encode() if body is not None else None,
        method=method,
        headers={
            "Authorization": "Bearer " + token,
            "Accept": "application/json",
            "Content-Type": "application/json",
        },
    )
    try:
        response = urllib.request.urlopen(request, timeout=20)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        data = response.read()
        return response.status, json.loads(data) if data else None, response.headers


def mysql_scalar(url, statement):
    client = os.environ.get("RIZZY_TEST_MYSQL_CLIENT")
    if not client:
        return None
    parsed = urlparse(url)
    client_env = {**os.environ, "MYSQL_PWD": unquote(parsed.password or "")}
    result = subprocess.run(
        [client, "--no-defaults", "--host=" + (parsed.hostname or "localhost"),
         "--port=" + str(parsed.port or 3306),
         "--user=" + unquote(parsed.username or ""),
         "--database=" + parsed.path.lstrip("/"),
         "--skip-column-names", "--execute=" + statement],
        env=client_env, check=True, capture_output=True, text=True,
    )
    return result.stdout.strip()


def main():
    url = os.environ.get("RIZZY_TEST_MYSQL_URL")
    if not url or not url.startswith("mysql:"):
        raise SystemExit("Set RIZZY_TEST_MYSQL_URL to a dedicated MySQL database URL")
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-cluster-") as temp:
        env = {
            **os.environ,
            "RIZZY_DATABASE_URL": url,
            "RIZZY_SEED_EMAIL": "mysql-cluster-" + uuid.uuid4().hex + "@example.test",
            "RIZZY_STORAGE_DIR": str(Path(temp) / "storage"),
            "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
            "RIZZY_DISABLE_PUSH_DELIVERY": "1",
        }
        initializers = [
            subprocess.Popen(
                [BINARY, "init-db"], cwd=ROOT, env=env,
                stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True,
            )
            for _ in range(2)
        ]
        for process in initializers:
            _, error = process.communicate(timeout=30)
            assert process.returncode == 0, error[-1200:]
        seed = subprocess.run(
            [BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True
        ).stdout
        account = re.search(r"Account: (\d+)", seed).group(1)
        token = re.search(r"Bearer token: (\w+)", seed).group(1)
        processes = []
        bases = []
        try:
            for index in range(4):
                port = free_port()
                local_env = {**env, "RIZZY_BIND": f"127.0.0.1:{port}"}
                log = (Path(temp) / f"rizzy-{index}.log").open("w")
                process = subprocess.Popen(
                    [BINARY], cwd=ROOT, env=local_env, stdout=log,
                    stderr=subprocess.STDOUT, start_new_session=True,
                )
                processes.append((process, log))
                bases.append(f"http://127.0.0.1:{port}")
            for base, (process, log) in zip(bases, processes):
                for _ in range(100):
                    if process.poll() is not None:
                        raise AssertionError("Rizzy exited: " + Path(log.name).read_text()[-1200:])
                    try:
                        with urllib.request.urlopen(base + "/up", timeout=.2) as response:
                            if response.status == 200:
                                break
                    except OSError:
                        time.sleep(.1)
                else:
                    raise AssertionError("Rizzy did not start: " + Path(log.name).read_text()[-1200:])
            status, board, _ = call(
                bases[0], account, token, "/boards", "POST", {"board": {"name": "Cluster"}}
            )
            assert status == 201, (status, board)
            status, column, _ = call(
                bases[0], account, token, f"/boards/{board['id']}/columns", "POST",
                {"column": {"name": "In Progress"}}
            )
            assert status == 201, (status, column)
            status, destination, _ = call(
                bases[0], account, token, "/boards", "POST",
                {"board": {"name": "Cluster destination"}}
            )
            assert status == 201, (status, destination)

            status, columns_board, _ = call(
                bases[0], account, token, "/boards", "POST",
                {"board": {"name": "Concurrent columns"}}
            )
            assert status == 201, (status, columns_board)

            def create_column(index):
                return call(
                    bases[index % 4], account, token,
                    f"/boards/{columns_board['id']}/columns", "POST",
                    {"column": {"name": f"Column {index:02d}"}},
                )

            with concurrent.futures.ThreadPoolExecutor(max_workers=20) as pool:
                created_columns = list(pool.map(create_column, range(20)))
            failures = [(index, status, value) for index, (status, value, _)
                        in enumerate(created_columns) if status != 201]
            assert not failures, failures[:5]
            assert mysql_scalar(
                url, "SELECT GROUP_CONCAT(position ORDER BY position) FROM columns "
                f"WHERE board_id = '{columns_board['id']}'"
            ) == ",".join(str(position) for position in range(1, 21))
            status, ordered_columns, _ = call(
                bases[0], account, token, f"/boards/{columns_board['id']}/columns")
            assert status == 200 and len(ordered_columns) == 20

            def move_created_column(index, direction):
                column_id = ordered_columns[index]["id"]
                return call(bases[index % 4], account, token,
                            f"/columns/{column_id}/{direction}_position.json", "POST")

            with concurrent.futures.ThreadPoolExecutor(max_workers=10) as pool:
                moved_right = list(pool.map(
                    lambda index: move_created_column(index, "right"), range(0, 20, 2)))
            assert all(status == 201 for status, _, _ in moved_right), moved_right
            expected_swapped = ",".join(
                ordered_columns[index]["name"] for pair in range(0, 20, 2)
                for index in (pair + 1, pair))
            assert mysql_scalar(
                url, "SELECT GROUP_CONCAT(name ORDER BY position) FROM columns "
                f"WHERE board_id = '{columns_board['id']}'"
            ) == expected_swapped
            with concurrent.futures.ThreadPoolExecutor(max_workers=10) as pool:
                moved_left = list(pool.map(
                    lambda index: move_created_column(index, "left"), range(0, 20, 2)))
            assert all(status == 201 for status, _, _ in moved_left), moved_left
            assert mysql_scalar(
                url, "SELECT GROUP_CONCAT(name ORDER BY position) FROM columns "
                f"WHERE board_id = '{columns_board['id']}'"
            ) == ",".join(column["name"] for column in ordered_columns)

            def create(index):
                return call(
                    bases[index % 4], account, token, f"/boards/{board['id']}/cards", "POST",
                    {"card": {"title": f"Concurrent {index}"}},
                )

            with concurrent.futures.ThreadPoolExecutor(max_workers=32) as pool:
                results = list(pool.map(create, range(400)))
            failures = [(index, status, value) for index, (status, value, _) in enumerate(results)
                        if status != 201]
            assert not failures, failures[:5]
            numbers = [value["number"] for _, value, _ in results]
            assert sorted(numbers) == list(range(1, 401)), numbers
            def create_step(index):
                number = results[index][1]["number"]
                return call(bases[index % 4], account, token,
                            f"/cards/{number}/steps", "POST",
                            {"step": {"content": f"Step {index}"}})

            with concurrent.futures.ThreadPoolExecutor(max_workers=32) as pool:
                steps = list(pool.map(create_step, range(400)))
            failures = [(index, status, value) for index, (status, value, _) in enumerate(steps)
                        if status != 201]
            assert not failures, failures[:5]
            assert len({value["id"] for _, value, _ in steps}) == 400

            def update_step(index):
                number = results[index][1]["number"]
                step_id = steps[index][1]["id"]
                return call(bases[index % 4], account, token,
                            f"/cards/{number}/steps/{step_id}", "PATCH",
                            {"step": {"content": f"Edited {index}", "completed": True}})

            with concurrent.futures.ThreadPoolExecutor(max_workers=32) as pool:
                updated = list(pool.map(update_step, range(400)))
            failures = [(index, status, value) for index, (status, value, _) in enumerate(updated)
                        if status != 200 or value["content"] != f"Edited {index}"
                        or value["completed"] is not True]
            assert not failures, failures[:5]

            def delete_step(index):
                number = results[index][1]["number"]
                step_id = steps[index][1]["id"]
                return call(bases[index % 4], account, token,
                            f"/cards/{number}/steps/{step_id}", "DELETE")

            with concurrent.futures.ThreadPoolExecutor(max_workers=32) as pool:
                deleted = list(pool.map(delete_step, range(400)))
            failures = [(index, status, value) for index, (status, value, _) in enumerate(deleted)
                        if status != 204]
            assert not failures, failures[:5]

            # Step insertion takes a foreign-key lock on its card. In parallel,
            # a title edit updates that card and touches the shared board. The
            # two requests use different app processes for each card.
            def mixed_write(item):
                index, kind = item
                number = results[index][1]["number"]
                base = bases[(2 * index + kind) % 4]
                if kind == 0:
                    return call(base, account, token, f"/cards/{number}/steps", "POST",
                                {"step": {"content": f"Mixed {index}"}})
                return call(base, account, token, f"/cards/{number}", "PATCH",
                            {"card": {"title": f"Mixed title {index}"}})

            mixed_items = [(index, kind) for index in range(200) for kind in (0, 1)]
            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                mixed = list(pool.map(mixed_write, mixed_items))
            failures = [(item, status, value) for item, (status, value, _) in zip(mixed_items, mixed)
                        if status != (201 if item[1] == 0 else 200)]
            assert not failures, failures[:5]
            assert len({value["id"] for (index, kind), (_, value, _) in zip(mixed_items, mixed)
                        if kind == 0}) == 200
            assert all(value["title"] == f"Mixed title {index}"
                       for (index, kind), (_, value, _) in zip(mixed_items, mixed) if kind == 1)

            def mixed_watch_write(item):
                index, kind = item
                number = results[index + 200][1]["number"]
                base = bases[(2 * index + kind) % 4]
                if kind == 0:
                    return call(base, account, token, f"/cards/{number}/watch", "DELETE")
                return call(base, account, token, f"/cards/{number}", "PATCH",
                            {"card": {"title": f"Watch title {index}"}})

            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                watch_mixed = list(pool.map(mixed_watch_write, mixed_items))
            failures = [(item, status, value) for item, (status, value, _) in zip(mixed_items, watch_mixed)
                        if status != (204 if item[1] == 0 else 200)]
            assert not failures, failures[:5]

            def mixed_triage_write(item):
                index, kind = item
                number = results[index][1]["number"]
                base = bases[(2 * index + kind) % 4]
                if kind == 0:
                    return call(base, account, token, f"/cards/{number}/triage", "POST",
                                {"column_id": column["id"]})
                return call(base, account, token, f"/cards/{number}", "PATCH",
                            {"card": {"title": f"Triage title {index}"}})

            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                triage_mixed = list(pool.map(mixed_triage_write, mixed_items))
            failures = [(item, status, value) for item, (status, value, _) in zip(mixed_items, triage_mixed)
                        if status != (204 if item[1] == 0 else 200)]
            assert not failures, failures[:5]

            def mixed_reaction_write(item):
                index, kind = item
                number = results[index + 200][1]["number"]
                base = bases[(2 * index + kind) % 4]
                if kind == 0:
                    return call(base, account, token, f"/cards/{number}/reactions", "POST",
                                {"reaction": {"content": "🎉"}})
                return call(base, account, token, f"/cards/{number}", "PATCH",
                            {"card": {"title": f"Reaction title {index}"}})

            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                reaction_mixed = list(pool.map(mixed_reaction_write, mixed_items))
            failures = [(item, status, value) for item, (status, value, _) in zip(mixed_items, reaction_mixed)
                        if status != (201 if item[1] == 0 else 200)]
            assert not failures, failures[:5]
            reaction_ids = {
                index: value["id"]
                for (index, kind), (_, value, _) in zip(mixed_items, reaction_mixed)
                if kind == 0
            }

            def mixed_reaction_delete(item):
                index, kind = item
                number = results[index + 200][1]["number"]
                base = bases[(2 * index + kind) % 4]
                if kind == 0:
                    return call(base, account, token,
                                f"/cards/{number}/reactions/{reaction_ids[index]}", "DELETE")
                return call(base, account, token, f"/cards/{number}", "PATCH",
                            {"card": {"title": f"Reaction removed title {index}"}})

            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                reaction_deleted = list(pool.map(mixed_reaction_delete, mixed_items))
            failures = [(item, status, value)
                        for item, (status, value, _) in zip(mixed_items, reaction_deleted)
                        if status != (204 if item[1] == 0 else 200)]
            assert not failures, failures[:5]

            def mixed_comment_write(item, operation):
                index, kind = item
                number = results[index + 200][1]["number"]
                base = bases[(2 * index + kind) % 4]
                if kind == 1:
                    return call(base, account, token, f"/cards/{number}", "PATCH",
                                {"card": {"title": f"Comment {operation} title {index}"}})
                if operation == "create":
                    return call(base, account, token, f"/cards/{number}/comments", "POST",
                                {"comment": {"body": f"Concurrent comment {index}"}})
                path = f"/cards/{number}/comments/{comment_ids[index]}"
                if operation == "edit":
                    return call(base, account, token, path, "PATCH",
                                {"comment": {"body": f"Edited concurrent comment {index}"}})
                return call(base, account, token, path, "DELETE")

            comment_ids = {}
            for operation, expected in (("create", 201), ("edit", 200), ("delete", 204)):
                with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                    comment_mixed = list(pool.map(
                        lambda item: mixed_comment_write(item, operation), mixed_items))
                failures = [(item, status, value)
                            for item, (status, value, _) in zip(mixed_items, comment_mixed)
                            if status != (expected if item[1] == 0 else 200)]
                assert not failures, (operation, failures[:5])
                if operation == "create":
                    comment_ids = {
                        index: value["id"]
                        for (index, kind), (_, value, _) in zip(mixed_items, comment_mixed)
                        if kind == 0
                    }
                    assert len(set(comment_ids.values())) == 200
                elif operation == "edit":
                    assert all(value["body"]["plain_text"] ==
                               f"Edited concurrent comment {index}"
                               for (index, kind), (_, value, _) in zip(mixed_items, comment_mixed)
                               if kind == 0)
            for index in (0, 99, 199):
                number = results[index + 200][1]["number"]
                status, _, _ = call(bases[index % 4], account, token,
                                    f"/cards/{number}/comments/{comment_ids[index]}")
                assert status == 404, (index, status)
            if os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
                deleted_ids = ",".join(f"'{comment_ids[index]}'" for index in range(200))
                assert mysql_scalar(url, f"SELECT COUNT(*) FROM comments WHERE id IN ({deleted_ids})") == "0"

            def mixed_move_write(item):
                index, kind = item
                number = results[index][1]["number"]
                base = bases[(2 * index + kind) % 4]
                if kind == 0:
                    return call(base, account, token, f"/cards/{number}/board", "PATCH",
                                {"board_id": destination["id"]})
                return call(base, account, token, f"/cards/{number}", "PATCH",
                            {"card": {"title": f"Moved title {index}"}})

            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                move_mixed = list(pool.map(mixed_move_write, mixed_items))
            failures = [(item, status, value) for item, (status, value, _) in zip(mixed_items, move_mixed)
                        if status != 200]
            assert not failures, failures[:5]
            assert all(value["board"]["id"] == destination["id"]
                       for (index, kind), (_, value, _) in zip(mixed_items, move_mixed) if kind == 0)
            for index in (0, 99, 199):
                number = results[index][1]["number"]
                status, moved, _ = call(bases[index % 4], account, token, f"/cards/{number}")
                assert status == 200 and moved["board"]["id"] == destination["id"]
                assert moved["title"] == f"Moved title {index}"

            def reverse_move(index):
                from_destination = index < 100
                card_index = index if from_destination else index + 100
                number = results[card_index][1]["number"]
                next_board = board["id"] if from_destination else destination["id"]
                return call(bases[index % 4], account, token,
                            f"/cards/{number}/board", "PATCH", {"board_id": next_board})

            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                reverse = list(pool.map(reverse_move, range(200)))
            failures = [(index, status, value) for index, (status, value, _) in enumerate(reverse)
                        if status != 200]
            assert not failures, failures[:5]
            assert all(value["board"]["id"] == (board["id"] if index < 100 else destination["id"])
                       for index, (_, value, _) in enumerate(reverse))

            def mixed_watch_move(item):
                index, kind = item
                card_index = index if index < 100 else index + 100
                number = results[card_index][1]["number"]
                base = bases[(2 * index + kind) % 4]
                if kind == 0:
                    return call(base, account, token, f"/cards/{number}/watch",
                                "DELETE" if index < 100 else "POST")
                next_board = destination["id"] if index < 100 else board["id"]
                return call(base, account, token, f"/cards/{number}/board", "PATCH",
                            {"board_id": next_board})

            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                watch_move = list(pool.map(mixed_watch_move, mixed_items))
            failures = [(item, status, value)
                        for item, (status, value, _) in zip(mixed_items, watch_move)
                        if status != (204 if item[1] == 0 else 200)]
            assert not failures, failures[:5]
            assert all(value["board"]["id"] ==
                       (destination["id"] if index < 100 else board["id"])
                       for (index, kind), (_, value, _) in zip(mixed_items, watch_move)
                       if kind == 1)
            if os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
                for index in (0, 99, 100, 199):
                    card_index = index if index < 100 else index + 100
                    card_id = results[card_index][1]["id"]
                    watching = mysql_scalar(
                        url, "SELECT watching FROM watches WHERE "
                             f"card_id='{card_id}' AND user_id="
                             f"(SELECT creator_id FROM cards WHERE id='{card_id}')",
                    )
                    assert watching == ("0" if index < 100 else "1"), index

            status, users, _ = call(bases[0], account, token, "/users")
            assert status == 200, status
            owner_id = next(user["id"] for user in users if user["role"] == "owner")

            def mixed_assignment_write(item):
                index, kind = item
                number = results[index][1]["number"]
                base = bases[(2 * index + kind) % 4]
                if kind == 0:
                    return call(base, account, token, f"/cards/{number}/assignments", "POST",
                                {"assignee_id": owner_id})
                return call(base, account, token, f"/cards/{number}", "PATCH",
                            {"card": {"title": f"Assigned title {index}"}})

            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                assignment_mixed = list(pool.map(mixed_assignment_write, mixed_items))
            failures = [(item, status, value) for item, (status, value, _) in zip(mixed_items, assignment_mixed)
                        if status != (204 if item[1] == 0 else 200)]
            assert not failures, failures[:5]

            def mixed_tag_write(item):
                index, kind = item
                number = results[index + 200][1]["number"]
                base = bases[(2 * index + kind) % 4]
                if kind == 0:
                    return call(base, account, token, f"/cards/{number}/taggings", "POST",
                                {"tag_title": "cluster-tag"})
                return call(base, account, token, f"/cards/{number}", "PATCH",
                            {"card": {"title": f"Tagged title {index}"}})

            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                tag_mixed = list(pool.map(mixed_tag_write, mixed_items))
            failures = [(item, status, value) for item, (status, value, _) in zip(mixed_items, tag_mixed)
                        if status != (204 if item[1] == 0 else 200)]
            assert not failures, failures[:5]
            for index in (0, 99, 199):
                assigned_number = results[index][1]["number"]
                status, assigned, _ = call(bases[index % 4], account, token,
                                            f"/cards/{assigned_number}")
                assert status == 200 and assigned["title"] == f"Assigned title {index}"
                assert any(user["id"] == owner_id for user in assigned["assignees"])
                tagged_number = results[index + 200][1]["number"]
                status, tagged, _ = call(bases[index % 4], account, token,
                                          f"/cards/{tagged_number}")
                assert status == 200 and tagged["title"] == f"Tagged title {index}"
                assert "cluster-tag" in tagged["tags"]

            def double_assignment_toggle(item):
                index, kind = item
                number = results[index][1]["number"]
                return call(bases[(2 * index + kind) % 4], account, token,
                            f"/cards/{number}/assignments", "POST",
                            {"assignee_id": owner_id})

            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                assignment_toggled = list(pool.map(double_assignment_toggle, mixed_items))
            failures = [(item, status, value)
                        for item, (status, value, _) in zip(mixed_items, assignment_toggled)
                        if status != 204]
            assert not failures, failures[:5]
            for index in (0, 99, 199):
                number = results[index][1]["number"]
                status, assigned, _ = call(bases[index % 4], account, token,
                                            f"/cards/{number}")
                assert status == 200 and any(user["id"] == owner_id
                                             for user in assigned["assignees"]), index
            if os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
                card_ids = ",".join(f"'{results[index][1]['id']}'" for index in range(200))
                assignment_count = mysql_scalar(
                    url, "SELECT COUNT(*) FROM assignments WHERE "
                         f"assignee_id='{owner_id}' AND card_id IN ({card_ids})")
                assert assignment_count == "200", assignment_count

            def double_tag_toggle(item):
                index, kind = item
                number = results[index + 200][1]["number"]
                return call(bases[(2 * index + kind) % 4], account, token,
                            f"/cards/{number}/taggings", "POST",
                            {"tag_title": "cluster-tag"})

            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                toggled = list(pool.map(double_tag_toggle, mixed_items))
            failures = [(item, status, value) for item, (status, value, _) in zip(mixed_items, toggled)
                        if status != 204]
            assert not failures, failures[:5]
            for index in range(200):
                number = results[index + 200][1]["number"]
                status, tagged, _ = call(bases[index % 4], account, token, f"/cards/{number}")
                assert status == 200 and "cluster-tag" in tagged["tags"], (index, status, tagged)

            def mixed_comment_move(item):
                index, kind = item
                number = results[index + 200][1]["number"]
                base = bases[(2 * index + kind) % 4]
                if kind == 0:
                    return call(base, account, token, f"/cards/{number}/comments", "POST",
                                {"comment": {"body": f"Moving comment {index}"}})
                return call(base, account, token, f"/cards/{number}/board", "PATCH",
                            {"board_id": destination["id"]})

            with concurrent.futures.ThreadPoolExecutor(max_workers=64) as pool:
                comment_move = list(pool.map(mixed_comment_move, mixed_items))
            failures = [(item, status, value)
                        for item, (status, value, _) in zip(mixed_items, comment_move)
                        if status != (201 if item[1] == 0 else 200)]
            assert not failures, ("comment/move", failures[:5])
            moved_comment_ids = [comment_move[2 * index][1]["id"] for index in range(200)]
            assert len(set(moved_comment_ids)) == 200
            if os.environ.get("RIZZY_TEST_MYSQL_CLIENT"):
                moved_ids = ",".join(f"'{comment_id}'" for comment_id in moved_comment_ids)
                assert mysql_scalar(url, f"SELECT COUNT(*) FROM comments WHERE id IN ({moved_ids})") == "200"
                assert mysql_scalar(url, "SELECT COUNT(*) FROM events "
                                     "WHERE eventable_type='Comment' AND "
                                     f"eventable_id IN ({moved_ids})") == "200"
            for index in (0, 99, 199):
                number = results[index + 200][1]["number"]
                comment = comment_move[2 * index][1]
                status, shown, _ = call(bases[index % 4], account, token,
                                        f"/cards/{number}/comments/{comment['id']}")
                assert status == 200 and shown["body"]["plain_text"] == f"Moving comment {index}"
                status, moved, _ = call(bases[index % 4], account, token, f"/cards/{number}")
                assert status == 200 and moved["board"]["id"] == destination["id"]

            status, cards, headers = call(bases[1], account, token, "/cards")
            assert status == 200 and len(cards) == 15, (status, len(cards))
            assert int(headers["X-Total-Count"]) == 400, headers
            print("MySQL cluster passed: 20 concurrent columns and 20 position swaps, 400 unique cards, mixed writes including comment CRUD and move, reaction deletion and watch/move races, opposite-direction moves, and 200 double assignment and tag toggles across four processes")
        except Exception:
            for index, (_, log) in enumerate(processes):
                log.flush()
                print(f"rizzy-{index} log tail: {Path(log.name).read_text()[-2000:]}",
                      file=sys.stderr)
            raise
        finally:
            for process, log in processes:
                if process.poll() is None:
                    os.killpg(process.pid, signal.SIGTERM)
                process.wait(timeout=10)
                log.close()


if __name__ == "__main__":
    main()
