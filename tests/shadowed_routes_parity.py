#!/usr/bin/env python3
"""Compare methods that Rails routes through a dynamic path behind a literal path."""

import os
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from http_methods_parity import probe  # noqa: E402
from route_get_status_parity import NoRedirect  # noqa: E402
from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402


SHADOWED = {
    "/boards/new": ("PATCH", "PUT", "DELETE"),
    "/cards/previews": ("PATCH", "PUT", "DELETE"),
    "/filters/settings_refresh": ("DELETE",),
    "/users/joins": ("GET", "HEAD", "PATCH", "PUT", "DELETE"),
    "/users/verifications": ("GET", "HEAD", "PATCH", "PUT", "DELETE"),
    "/my/access_tokens/new": ("DELETE",),
}


def main():
    cases = [(method, path + suffix, accept, bearer)
             for path, methods in SHADOWED.items()
             for method in methods
             for suffix in ("", ".json")
             for accept in ("application/json", "text/html")
             for bearer in (False, True)]
    unscoped_cases = [("DELETE", "/my/access_tokens/new" + suffix, accept, bearer)
                      for suffix in ("", ".json")
                      for accept in ("application/json", "text/html")
                      for bearer in (False, True)]
    with tempfile.TemporaryDirectory(prefix="rizzy-shadowed-routes-") as temporary:
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
            for case in cases:
                source = probe(targets["fizzy"], *case)
                rust = probe(targets["rizzy"], *case)
                assert source == rust, (case, source[:2], rust[:2], source[3:], rust[3:],
                                        source[2][:120], rust[2][:120])
            for case in unscoped_cases:
                source = probe(targets["fizzy"], *case, scoped=False)
                rust = probe(targets["rizzy"], *case, scoped=False)
                assert source == rust, (case, source[:2], rust[:2], source[3:], rust[3:],
                                        source[2][:120], rust[2][:120])
        finally:
            for process in reversed(processes):
                stop_server(process)
    print(f"Matched pinned Fizzy on {len(cases) + len(unscoped_cases)} shadowed route requests")


if __name__ == "__main__":
    main()
