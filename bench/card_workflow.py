#!/usr/bin/env python3
"""Equal-count create/edit/comment/close/read workload on disposable accounts."""
import argparse
import concurrent.futures
import json
import os
import sys
import time
import urllib.error
import urllib.parse
import urllib.request

from compare import pct, process_sample, shape


STEPS = ("create", "update", "comment", "close", "read")


def api(target, path, method="GET", payload=None):
    body = json.dumps(payload).encode() if payload is not None else None
    request = urllib.request.Request(
        f"{target['url']}/{target['account']}{path}", body, method=method,
        headers={"Authorization": f"Bearer {target['token']}",
                 "Accept": "application/json", "Content-Type": "application/json"},
    )
    try:
        with urllib.request.urlopen(request, timeout=30) as response:
            raw = response.read()
            return response.status, (json.loads(raw) if raw else None), response.headers
    except urllib.error.HTTPError as error:
        raise RuntimeError(f"{method} {path}: HTTP {error.code}: {error.read(200)!r}") from error


def expect(target, path, method="GET", payload=None, status=200):
    actual, body, headers = api(target, path, method, payload)
    assert actual == status, (method, path, actual, status)
    return body, headers


def workflow(target, board_id, index, capture_shapes=False):
    title = f"Workflow card {index:06d}"
    revised = title + " revised"
    timings = {}

    def step(name, path, method="GET", payload=None, status=200):
        started = time.perf_counter()
        result = expect(target, path, method, payload, status)
        timings[name] = time.perf_counter() - started
        return result[0]

    card = step("create", f"/boards/{board_id}/cards", "POST",
                {"card": {"title": title, "description": "Initial body"}}, 201)
    number = card["number"]
    assert card["title"] == title and card["description"] == "Initial body"
    updated = step("update", f"/cards/{number}", "PATCH",
                   {"card": {"title": revised, "description": "Revised body"}})
    assert updated["title"] == revised and updated["description"] == "Revised body"
    comment = step("comment", f"/cards/{number}/comments", "POST",
                   {"comment": {"body": f"Workflow comment {index:06d}"}}, 201)
    assert comment["body"]["plain_text"] == f"Workflow comment {index:06d}"
    closed = step("close", f"/cards/{number}/closure", "POST", status=204)
    assert closed is None
    final = step("read", f"/cards/{number}")
    assert final["title"] == revised and final["description"] == "Revised body" and final["closed"] is True
    result = {"number": number, "timings": timings}
    if capture_shapes:
        result["shapes"] = {
            "create": shape(card), "update": shape(updated),
            "comment": shape(comment), "read": shape(final),
        }
    return result


def preflight(target):
    board, _ = expect(target, "/boards", "POST", {"board": {"name": "Workflow parity preflight"}}, 201)
    try:
        result = workflow(target, board["id"], 0, capture_shapes=True)
        comments, headers = expect(target, f"/cards/{result['number']}/comments")
        assert int(headers["X-Total-Count"]) == 3 and len(comments) == 3
        assert any(comment["body"]["plain_text"] == "Workflow comment 000000" for comment in comments)
        return result["shapes"]
    finally:
        expect(target, f"/boards/{board['id']}", "DELETE", status=204)


def benchmark(target, count, concurrency, round_number):
    board, _ = expect(target, "/boards", "POST",
                      {"board": {"name": f"Workflow benchmark round {round_number} {time.time_ns()}"}}, 201)
    try:
        return benchmark_board(target, count, concurrency, board["id"])
    finally:
        expect(target, f"/boards/{board['id']}", "DELETE", status=204)


def benchmark_board(target, count, concurrency, board_id):
    before = process_sample(target.get("pid")) if target.get("pid") else None
    started = time.monotonic()

    def run(index):
        try:
            return workflow(target, board_id, index), None
        except Exception as error:
            return None, f"{type(error).__name__}: {str(error)[:180]}"

    with concurrent.futures.ThreadPoolExecutor(max_workers=concurrency) as pool:
        outcomes = list(pool.map(run, range(count)))
    elapsed = time.monotonic() - started
    after = process_sample(target.get("pid")) if target.get("pid") else None
    completed = [result for result, error in outcomes if error is None]
    errors = [error for _, error in outcomes if error is not None]
    numbers = [result["number"] for result in completed]
    assert not errors, errors[:10]
    assert len(numbers) == count and len(set(numbers)) == count

    query = urllib.parse.urlencode({"board_ids[]": board_id})
    cards, card_headers = expect(target, f"/cards?{query}&indexed_by=closed")
    activities, activity_headers = expect(target, f"/activities?{query}")
    card_total = int(card_headers["X-Total-Count"])
    event_total = int(activity_headers["X-Total-Count"])
    assert card_total == count and event_total == 4 * count, (card_total, event_total, count)
    assert cards and all(card["closed"] for card in cards)
    assert activities and {event["action"] for event in activities} <= {
        "card_published", "card_title_changed", "comment_created", "card_closed"}
    comment_sample, comment_headers = expect(target, f"/cards/{numbers[0]}/comments")
    assert len(comment_sample) == int(comment_headers["X-Total-Count"]) == 3
    step_samples = {step: [result["timings"][step] for result in completed] for step in STEPS}
    workflow_samples = [sum(result["timings"].values()) for result in completed]
    data = {
        "workflows": count, "concurrency": concurrency, "seconds": round(elapsed, 2),
        "workflows_per_second": round(count / elapsed, 1),
        "p50_workflow_ms": pct(workflow_samples, .5),
        "p95_workflow_ms": pct(workflow_samples, .95),
        "p95_steps_ms": {step: pct(step_samples[step], .95) for step in STEPS},
        "http_errors": 0, "unique_numbers": count,
        "verified_cards": card_total, "verified_events": event_total,
        "verified_comment_sample": len(comment_sample),
    }
    if before and after:
        data["server_resources"] = {
            "cpu_seconds": round(after["cpu_seconds"] - before["cpu_seconds"], 2),
            "cpu_cores_average": round((after["cpu_seconds"] - before["cpu_seconds"]) / elapsed, 2),
            "pss_mib_before": round(before["pss_mib"], 1),
            "pss_mib_after": round(after["pss_mib"], 1),
            "write_mib": round(after["write_mib"] - before["write_mib"], 1),
        }
    return data


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--workflows", type=int, default=100)
    parser.add_argument("--concurrency", default="8,32")
    parser.add_argument("--rounds", type=int, default=2)
    args = parser.parse_args()
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        required = ("URL", "ACCOUNT", "TOKEN")
        if not all(os.getenv(f"{name}_{field}") for field in required):
            parser.error(f"Set {name}_URL, {name}_ACCOUNT, and {name}_TOKEN")
        target = {field.lower(): os.environ[f"{name}_{field}"] for field in required}
        if os.getenv(f"{name}_PID"):
            target["pid"] = int(os.environ[f"{name}_PID"])
        targets[name.lower()] = target
    shapes = {name: preflight(target) for name, target in targets.items()}
    assert shapes["fizzy"] == shapes["rizzy"], "Preflight JSON response shapes differ"
    result = {"workload": "create, edit title and description, comment, close, read",
              "preflight_shapes_equal": True, "workflows_per_run": args.workflows,
              "rounds": args.rounds, "runs": []}
    for round_number in range(args.rounds):
        names = ("fizzy", "rizzy") if round_number % 2 == 0 else ("rizzy", "fizzy")
        for concurrency in map(int, args.concurrency.split(",")):
            for name in names:
                sample = benchmark(targets[name], args.workflows, concurrency, round_number + 1)
                result["runs"].append({"round": round_number + 1, "target": name, **sample})
                print(f"round {round_number + 1} {name} c={concurrency}: {sample['workflows_per_second']} workflows/s, p95 {sample['p95_workflow_ms']} ms", file=sys.stderr, flush=True)
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
