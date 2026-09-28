#!/usr/bin/env python3
"""Compare pinned Fizzy's dispatchable route methods with Rizzy's router.

This checks route registration only. Passing does not establish response parity.
Regenerate source_route_inventory.json from the pinned source with:
  bin/rails runner /absolute/path/to/tests/source_route_inventory.rb
"""

import argparse
import json
import re
import subprocess
from collections import defaultdict
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
VERBS = {"get", "post", "put", "patch", "delete", "head"}


def route_calls(source):
    start = 0
    while (index := source.find(".route(", start)) >= 0:
        cursor, depth, quoted, escaped = index + len(".route("), 1, False, False
        while cursor < len(source) and depth:
            char = source[cursor]
            if quoted:
                if escaped:
                    escaped = False
                elif char == "\\":
                    escaped = True
                elif char == '"':
                    quoted = False
            elif char == '"':
                quoted = True
            elif char == "(":
                depth += 1
            elif char == ")":
                depth -= 1
            cursor += 1
        call = source[index:cursor]
        match = re.match(r'\.route\(\s*"([^"]+)"', call)
        if match:
            methods = {name.upper() for name in
                       re.findall(r'(?<![A-Za-z_])(get|post|put|patch|delete|head)\s*\(', call)}
            methods.update(name.upper() for name in
                           re.findall(r'\.([a-z]+)\s*\(', call) if name in VERBS)
            yield match.group(1), methods
        start = cursor


def normalized(path):
    path = path.removeprefix("/{account}").replace("(.:format)", "").replace(".json", "")
    if path.startswith("/client_configurations("):
        return "/client_configurations/*"
    return re.sub(r'\{[^}]+\}|:[A-Za-z_]+|\*[A-Za-z_]+', "*", path)


def category(controller):
    if controller.startswith("turbo/native/"):
        return "turbo_native"
    if controller.startswith("active_storage/"):
        return "active_storage"
    if controller.startswith(("action_mailbox/", "rails/conductor/action_mailbox/")):
        return "action_mailbox"
    return "fizzy"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--strict", action="store_true", help="exit nonzero for missing methods")
    args = parser.parse_args()
    inventory = json.loads((ROOT / "tests/source_route_inventory.json").read_text())
    revision = subprocess.check_output(["git", "rev-parse", "HEAD"],
                                       cwd=ROOT / "upstream", text=True).strip()
    assert inventory["source_revision"] == revision, (
        "route inventory revision", inventory["source_revision"], revision)
    rust = defaultdict(set)
    for path, methods in route_calls((ROOT / "src/main.rs").read_text()):
        rust[normalized(path)].update(methods)
    found, missing = defaultdict(list), defaultdict(list)
    for route in inventory["routes"]:
        group = category(route["controller"])
        path = normalized(route["path"])
        for method in route["verb"].split("|"):
            item = (method, route["path"], route["controller"] + "#" + route["action"])
            (found if method in rust[path] else missing)[group].append(item)
    for group in ("fizzy", "turbo_native", "active_storage", "action_mailbox"):
        print(f"{group}: {len(found[group])} registered, {len(missing[group])} missing")
        for method, path, action in missing[group]:
            print(f"  {method:6} {path} ({action})")
    if args.strict and any(missing.values()):
        raise SystemExit(1)


if __name__ == "__main__":
    main()
