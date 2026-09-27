#!/usr/bin/env python3
"""Run the passkey registration and sign-in flow on disposable paired apps."""

import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from sustained_mixed import (prepare_rust, prepare_source, rust_env,  # noqa: E402
                             start_server, stop_server)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-passkey-flow-") as temporary:
        processes = []
        targets = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                if name == "fizzy":
                    env, credentials = prepare_source(directory, 2)
                else:
                    env = rust_env(directory)
                    env["RIZZY_DEV_MAGIC_LINKS"] = "1"
                    env, credentials = prepare_rust(directory, env)
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                targets[name] = {"url": url, "directory": directory,
                                 "account": str(credentials["account"])}
            shared = {**os.environ,
                      "FIZZY_URL": targets["fizzy"]["url"],
                      "FIZZY_DB": str(targets["fizzy"]["directory"] / "db.sqlite3"),
                      "FIZZY_ACCOUNT": targets["fizzy"]["account"],
                      "FIZZY_EMAIL": "owner@example.com",
                      "RIZZY_URL": targets["rizzy"]["url"].replace("127.0.0.1", "localhost"),
                      "RIZZY_DB": str(targets["rizzy"]["directory"] / "rizzy.sqlite3"),
                      "RIZZY_ACCOUNT": targets["rizzy"]["account"],
                      "RIZZY_EMAIL": "owner@example.com"}
            outcomes = {}
            for name in ("fizzy", "rizzy"):
                result = subprocess.run([sys.executable, str(ROOT / "tests/passkey_parity.py")],
                                        cwd=ROOT, env={**shared, "PASSKEY_TARGET": name},
                                        capture_output=True, text=True, timeout=90)
                assert result.returncode == 0, (name, result.stdout[-1000:], result.stderr[-2000:])
                outcomes[name] = result.stdout
            assert outcomes["fizzy"] == outcomes["rizzy"], outcomes
        finally:
            for process in reversed(processes):
                stop_server(process)
    print("Pinned Fizzy and Rizzy passed matching passkey registration, sign-in, replay, and deletion flows")


if __name__ == "__main__":
    main()
