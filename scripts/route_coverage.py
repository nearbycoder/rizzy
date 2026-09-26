#!/usr/bin/env python3
"""Inventory application routes in pinned Fizzy and route declarations in Rizzy."""
import collections
import json
import os
import re
import subprocess
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
UPSTREAM = ROOT / "upstream"
RUBY = Path("/home/nearby/.local/rizzy-ruby")
BUNDLE = Path("/home/nearby/.local/rizzy-bundle")
OUTPUT = ROOT / "docs" / "route-coverage.json"
SUMMARY = ROOT / "ROUTE_COVERAGE.md"
VERBS = {"GET", "POST", "PUT", "PATCH", "DELETE"}


def upstream_routes():
    script = '''require "json"; routes = Rails.application.routes.routes.filter_map do |r|
      controller = r.defaults[:controller]
      next unless controller && File.exist?(Rails.root.join("app/controllers/#{controller}_controller.rb"))
      action = r.defaults[:action]
      klass = "#{controller.camelize}Controller".safe_constantize
      next unless klass && klass.action_methods.include?(action.to_s)
      [r.verb, r.path.spec.to_s, controller, action]
    end; puts JSON.generate(routes)'''
    env = {**os.environ, "PATH": str(RUBY / "bin") + ":" + os.environ.get("PATH", ""),
           "LD_LIBRARY_PATH": str(RUBY / "lib"), "BUNDLE_PATH": str(BUNDLE)}
    result = subprocess.run(["bin/rails", "runner", script], cwd=UPSTREAM, env=env,
                            capture_output=True, text=True, check=True)
    return [{"method": verb, "path": path, "controller": controller, "action": action}
            for verb, path, controller, action in json.loads(result.stdout) if verb in VERBS]


def route_call(source, start):
    opening = source.index("(", start)
    depth, quoted, escaped = 0, False, False
    for index in range(opening, len(source)):
        char = source[index]
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
            if depth == 0:
                return source[opening:index + 1]
    raise ValueError("Unclosed Rust route expression")


def rust_routes():
    source = (ROOT / "src" / "main.rs").read_text()
    routes = []
    for match in re.finditer(r'\.route\(\s*"([^"]+)"', source):
        expression = route_call(source, match.start())
        for verb in re.findall(r'(?<!\w)(get|post|put|patch|delete)\s*\(', expression):
            routes.append({"method": verb.upper(), "path": match.group(1)})
    return routes


def normalized(path):
    path = path.replace("(.:format)", "")
    path = path.replace("(/:platform)_v(:version)", "/{configuration}")
    path = re.sub(r'^/\{account\}(?=/|$)', '', path)
    path = re.sub(r'\{[^}]+\}|:[^/()]+|\*[^/()]+', ':*', path)
    return path or "/"


def main():
    reference = upstream_routes()
    rust = rust_routes()
    rust_keys = {(route["method"], normalized(route["path"])) for route in rust}
    for route in reference:
        route["route_present"] = (route["method"], normalized(route["path"])) in rust_keys
    OUTPUT.parent.mkdir(exist_ok=True)
    OUTPUT.write_text(json.dumps({"reference_revision": "477c943e0506f109e5bc83ae9dadbe519732c045",
                                  "reference_routes": reference, "rust_routes": rust}, indent=2) + "\n")
    counts = collections.defaultdict(lambda: [0, 0])
    for route in reference:
        bucket = counts[route["controller"]]
        bucket[0] += 1
        bucket[1] += route["route_present"]
    present = sum(route["route_present"] for route in reference)
    lines = ["# Application route inventory", "",
             "Pinned Fizzy revision: `477c943e0506f109e5bc83ae9dadbe519732c045`.", "",
             f"The source implements actions for {len(reference)} GET/POST/PUT/PATCH/DELETE application routes across {len(counts)} controllers. "
             f"Rizzy declares matching method and normalized path for {present}. "
             "This is a route-presence inventory only: a match does not prove response, permissions, side effects, HTML, or JSON parity. "
             "Optional format suffixes and parameter names are normalized; generated routes without a controller action and mounted engine routes are excluded.", "",
             "The parameterized client-configuration route is normalized as one path segment; its version and format behavior is compared with Fizzy in `tests/client_configuration_route_parity.py`. "
             "Three legacy `collections` redirects are generated routes and excluded from the table. Rizzy serves their root and account forms, including optional JSON suffixes and trailing slashes.", "",
             "| Source controller | Routes | Method/path present in Rust |", "| --- | ---: | ---: |"]
    for controller, (total, found) in sorted(counts.items(), key=lambda item: (item[1][1] - item[1][0], item[0])):
        lines.append(f"| `{controller}` | {total} | {found} |")
    lines.extend(["", "The machine-readable route list is in [docs/route-coverage.json](docs/route-coverage.json).",
                  "Run `python3 scripts/route_coverage.py` after route changes. Full parity requires the gates in [PARITY.md](PARITY.md).", ""])
    SUMMARY.write_text("\n".join(lines))
    print(f"{present}/{len(reference)} normalized route signatures present across {len(counts)} controllers")


if __name__ == "__main__":
    main()
