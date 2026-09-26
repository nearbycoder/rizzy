#!/usr/bin/env python3
"""Compare deployment CSP options against pinned Fizzy production responses."""

import os
import re
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from sustained_mixed import (  # noqa: E402
    prepare_rust, prepare_source, rust_env, source_env, start_server, stop_server,
)
from static_csp_parity import get  # noqa: E402

CASES = {
    "extra_sources": {
        "CSP_DEFAULT_SRC": "https://default.example",
        "CSP_SCRIPT_SRC": "https://scripts.example https://second.example",
        "CSP_STYLE_SRC": "https://styles.example",
        "CSP_CONNECT_SRC": "https://connect.example",
        "CSP_FRAME_SRC": "https://frames.example",
        "CSP_IMG_SRC": "https://images.example",
        "CSP_FONT_SRC": "https://fonts.example",
        "CSP_MEDIA_SRC": "https://media.example",
        "CSP_WORKER_SRC": "https://workers.example",
        "CSP_FRAME_ANCESTORS": "https://parent.example",
        "CSP_FORM_ACTION": "https://forms.example",
        "CSP_REPORT_URI": "https://reports.example/csp",
    },
    "report_only": {
        "CSP_SCRIPT_SRC": "https://scripts.example",
        "CSP_REPORT_URI": "https://reports.example/csp",
        "CSP_REPORT_ONLY": "true",
    },
    "disabled": {"DISABLE_CSP": "1"},
    "disabled_empty": {"DISABLE_CSP": ""},
}


def normalized_policy(headers):
    values = (headers.get("Content-Security-Policy"),
              headers.get("Content-Security-Policy-Report-Only"))
    assert not all(values), values
    return tuple(re.sub(r"'nonce-[^']+'", "'nonce-[NONCE]'", value) if value else None
                 for value in values)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    for case, options in CASES.items():
        with tempfile.TemporaryDirectory(prefix=f"rizzy-csp-{case}-") as temporary:
            processes = []
            results = {}
            try:
                for name in ("fizzy", "rizzy"):
                    directory = Path(temporary) / name
                    directory.mkdir()
                    environment = (source_env(directory, 2) if name == "fizzy"
                                   else rust_env(directory))
                    environment.update(options)
                    if name == "fizzy":
                        prepare_source(directory, 2, env=environment)
                    else:
                        prepare_rust(directory, env=environment)
                    process, url = start_server(name, directory, environment,
                                                sorted(os.sched_getaffinity(0))[:2])
                    processes.append(process)
                    responses = []
                    for path in ("/404.html", "/no-such-csp-page", "/up", "/session/new"):
                        status, headers, body = get(url, path)
                        responses.append((status, normalized_policy(headers)))
                        assert status in (200, 404), (case, name, path, status)
                        if not case.startswith("disabled") and path == "/session/new":
                            assert b"<script" in body
                    results[name] = responses
                assert results["fizzy"] == results["rizzy"], (case, results)
                print(f"{case}: static and dynamic CSP matches", flush=True)
            finally:
                for process in processes:
                    stop_server(process)


if __name__ == "__main__":
    main()
