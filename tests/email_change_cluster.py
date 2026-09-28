#!/usr/bin/env python3
"""Confirm signed email links across two Rizzy processes sharing SQLite."""

import http.cookiejar
import json
import os
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path[:0] = [str(ROOT / "bench"), str(ROOT / "tests")]

from draft_workflow_parity import NoRedirect, login, request  # noqa: E402
from sustained_mixed import (prepare_rust, rust_env, start_server,  # noqa: E402
                             stop_server)


def main():
    with tempfile.TemporaryDirectory(prefix="rizzy-email-cluster-") as temporary:
        directory = Path(temporary)
        env = rust_env(directory)
        env.pop("RIZZY_SECRET_KEY_BASE", None)
        env["RIZZY_DEV_MAGIC_LINKS"] = "1"
        env, credentials = prepare_rust(directory, env)
        cpus = sorted(os.sched_getaffinity(0))[:2]
        processes = []
        try:
            for name in ("process-a", "process-b"):
                log_directory = directory / name
                log_directory.mkdir()
                process, url = start_server("rizzy", log_directory, env, cpus)
                processes.append((process, url))
            source, destination = (item[1] for item in processes)
            jar = http.cookiejar.CookieJar()
            target = {"name": "rizzy", "url": source,
                      "account": credentials["account"],
                      "token": credentials["token"], "email": "owner@example.com",
                      "opener": urllib.request.build_opener(
                          urllib.request.HTTPCookieProcessor(jar), NoRedirect())}
            login(target)
            account = target["account"]
            status, _, body = request(target, f"/{account}/users", bearer=True)
            assert status == 200, status
            owner = next(user for user in json.loads(body) if user["role"] == "owner")
            path = f"/{account}/users/{owner['id']}/email_addresses"
            new_email = "other-process@example.com"
            status, headers, _ = request(target, path, "POST",
                                         {"email_address": new_email}, bearer=True)
            assert status == 201, status
            token = headers["X-Email-Change-Token"]
            target["url"] = destination
            confirmation = f"{path}/{token}/confirmation"
            status, _, _ = request(target, confirmation, "POST", b"", accept="application/json")
            assert status == 204, status
            target["url"] = source
            status, _, body = request(target, f"/{account}/users/{owner['id']}.json")
            assert status == 200 and json.loads(body)["email_address"] == new_email, status
            status, _, _ = request(target, confirmation, "POST", b"", accept="application/json")
            assert status == 422, status
            print("PASS email token issued on process A, confirmed on B, read on A")
        finally:
            for process, _ in reversed(processes):
                stop_server(process)


if __name__ == "__main__":
    main()
