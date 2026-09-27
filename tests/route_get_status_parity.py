#!/usr/bin/env python3
"""Probe pinned Fizzy GET routes with an absent resource on paired servers.

This is a routing/error-contract census, not a functional route parity test.
"""

import json
import re
import urllib.error
import urllib.request
from pathlib import Path

from route_coverage import category


ROOT = Path(__file__).resolve().parents[1]


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


OPENER = urllib.request.build_opener(NoRedirect())


def paths():
    inventory = json.loads((ROOT / "tests/source_route_inventory.json").read_text())
    assert inventory["source_revision"] == "477c943e0506f109e5bc83ae9dadbe519732c045"
    found = set()
    for route in inventory["routes"]:
        if "GET" not in route["verb"].split("|") or category(route["controller"]) != "fizzy":
            continue
        path = route["path"].removesuffix("(.:format)")
        if "(" in path or "*" in path or path in ("/up", "/manifest", "/service-worker"):
            continue
        path = re.sub(r":[A-Za-z_]+", "missing", path)
        found.add(path)
    return sorted(found)


def run(target):
    result = {}
    for path in paths():
        scoped = f"/{target['account']}{path}" if path != "/" else f"/{target['account']}/"
        request = urllib.request.Request(
            target["url"] + scoped,
            headers={"Authorization": "Bearer " + target["token"],
                     "Accept": "application/json"},
        )
        try:
            response = OPENER.open(request, timeout=12)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            response.read()
            result[path] = (response.status, response.headers.get_content_type())
    return result
