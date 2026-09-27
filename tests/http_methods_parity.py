#!/usr/bin/env python3
"""Compare HEAD and unsupported-method contracts with pinned Fizzy."""

import os
import sys
import tempfile
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from route_get_status_parity import NoRedirect, paths  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402


def probe(target, method, path, accept, bearer):
    scoped = f"/{target['account']}{path}" if path != "/" else f"/{target['account']}/"
    headers = {"Accept": accept}
    if bearer:
        headers["Authorization"] = "Bearer " + target["token"]
    request = urllib.request.Request(target["url"] + scoped, headers=headers, method=method)
    try:
        response = target["opener"].open(request, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        destination = urllib.parse.urlsplit(response.headers.get("Location", "")).path
        return (response.status, response.headers.get_content_type(), response.read(),
                response.headers.get("Allow"), destination)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-http-methods-") as temporary:
        processes = []
        targets = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                targets[name] = {"url": url, "account": str(credentials["account"]),
                                 "token": credentials["token"],
                                 "opener": urllib.request.build_opener(NoRedirect())}
            cases = [("HEAD", path, "application/json", True) for path in paths()]
            cases += [("OPTIONS", path, accept, bearer)
                      for path in paths()
                      for accept in ("application/json", "text/html")
                      for bearer in (False, True)]
            cases += [("OPTIONS", path + ".json", accept, bearer)
                      for path in ("/boards", "/cards/missing", "/my/identity",
                                   "/account/settings", "/notifications", "/search")
                      for accept in ("application/json", "text/html")
                      for bearer in (False, True)]
            unsupported = (("POST", "/activities"), ("DELETE", "/boards"),
                           ("PATCH", "/boards/new"), ("PUT", "/cards"),
                           ("TRACE", "/boards"), ("DELETE", "/my/menu"))
            cases += [(method, path, "application/json", True)
                      for method, path in unsupported]
            cases += [(method, path + ".json", "text/html", True)
                      for method, path in unsupported[:2]]
            for method, path, accept, bearer in cases:
                source = probe(targets["fizzy"], method, path, accept, bearer)
                rust = probe(targets["rizzy"], method, path, accept, bearer)
                assert source == rust, (method, path, accept, bearer, source[:2], rust[:2],
                                        source[3:], rust[3:], len(source[2]), len(rust[2]))
        finally:
            for process in reversed(processes):
                stop_server(process)
    print(f"Matched pinned Fizzy HEAD and unsupported-method contracts in {len(cases)} route cases")


if __name__ == "__main__":
    main()
