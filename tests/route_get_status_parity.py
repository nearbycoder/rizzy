#!/usr/bin/env python3
"""Probe pinned Fizzy GET routes with an absent resource on paired servers.

This is a routing/error-contract census, not a functional route parity test.
"""

import hashlib
import json
import re
import urllib.error
import urllib.parse
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


def run(target, detailed=False):
    result = {}
    route_paths = paths()
    for path in route_paths:
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
            body = response.read()
            content_type = response.headers.get_content_type()
            contract = [response.status, content_type]
            if detailed and 300 <= response.status < 400:
                location = urllib.parse.urlparse(response.headers.get("Location", ""))
                account_prefix = f"/{target['account']}/"
                destination_path = location.path.replace(account_prefix, "/:account/", 1)
                contract.append((destination_path, location.query))
            elif detailed and response.status >= 400:
                if content_type == "application/json":
                    try:
                        contract.append(json.loads(body))
                    except (json.JSONDecodeError, UnicodeDecodeError):
                        contract.append((len(body), hashlib.sha256(body).hexdigest()))
                else:
                    contract.append((len(body), hashlib.sha256(body).hexdigest()))
            result[path] = tuple(contract)
    assert len(result) == len(route_paths), (target["name"], len(result), len(route_paths))
    return result
