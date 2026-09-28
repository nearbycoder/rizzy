#!/usr/bin/env python3
"""Exercise Active Storage blob redirect aliases, proxy bytes, and ranges on both apps."""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from sustained_mixed import prepare_rust, prepare_source, start_server, stop_server  # noqa: E402
from card_image_parity import IMAGE, call  # noqa: E402
from embed_parity import upload  # noqa: E402


def check(target):
    account = target["account"]
    prefix = f"/{account}"
    status, _, raw = call(target, prefix + "/boards", "POST", {"board": {"name": "Storage proxy parity"}})
    assert status == 201, (target["name"], status, raw[:200])
    board = json.loads(raw)
    try:
        blob = upload(target)
        status, _, raw = call(target, prefix + f"/boards/{board['id']}/cards", "POST",
                              {"card": {"title": "Proxy file", "image": blob["signed_id"]}})
        assert status == 201, (target["name"], status, raw[:200])
        base = prefix + "/rails/active_storage/blobs"
        signed = blob["signed_id"] + "/moon.jpg"
        for path in (f"{base}/redirect/{signed}", f"{base}/{signed}"):
            status, headers, _ = call(target, path)
            assert status == 302 and "/rails/active_storage/" in headers["Location"], (
                target["name"], path, status, headers)
            status, headers, _ = call(target, path, bearer=False)
            assert status == 302 and headers["Location"].endswith("/session/new"), (
                target["name"], path, status, headers)

        proxy = f"{base}/proxy/{signed}"
        status, headers, content = call(target, proxy)
        assert status == 200 and content == IMAGE, (target["name"], status, len(content))
        assert headers.get_content_type() == "image/jpeg"
        assert headers["Content-Disposition"].startswith("inline")
        assert headers["Accept-Ranges"] == "bytes"
        private_cache = headers["Cache-Control"]
        assert "public" not in private_cache, (target["name"], private_cache)
        status, headers, content = call(target, proxy, headers={"Range": "bytes=1-3"})
        assert status == 206 and content == IMAGE[1:4], (target["name"], status, content)
        assert headers["Content-Range"] == f"bytes 1-3/{len(IMAGE)}"
        status, headers, content = call(target, proxy, headers={"Range": "bytes=-4"})
        assert status == 206 and content == IMAGE[-4:], (target["name"], status, content)
        status, headers, content = call(target, proxy + "?disposition=attachment")
        assert status == 200 and content == IMAGE
        assert headers["Content-Disposition"].startswith("attachment")
        status, headers, _ = call(target, proxy, bearer=False)
        assert status == 302 and headers["Location"].endswith("/session/new"), (
            target["name"], status, headers)

        status, _, _ = call(target, prefix + f"/boards/{board['id']}/publication", "POST")
        assert status == 201, (target["name"], status)
        status, headers, content = call(target, proxy, bearer=False)
        assert status == 200 and content == IMAGE, (target["name"], status)
        public_cache = headers["Cache-Control"]
        assert "public" in public_cache, (target["name"], public_cache)
        return {"proxy": 200, "range": 206, "anonymous_private": 302,
                "anonymous_public": 200, "alias": 302,
                "private_cache": private_cache, "public_cache": public_cache}
    finally:
        status, _, _ = call(target, prefix + f"/boards/{board['id']}", "DELETE")
        assert status == 204, (target["name"], status)


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-storage-proxy-") as temporary:
        processes = []
        results = {}
        try:
            for name in ("fizzy", "rizzy"):
                directory = Path(temporary) / name
                directory.mkdir()
                env, credentials = (prepare_source(directory, 2) if name == "fizzy"
                                    else prepare_rust(directory))
                process, url = start_server(name, directory, env,
                                            sorted(os.sched_getaffinity(0))[:2])
                processes.append(process)
                target = {"name": name, "url": url,
                          "account": str(credentials["account"]),
                          "token": credentials["token"]}
                results[name] = check(target)
                print(f"{name}: blob alias, proxy, ranges, and access passed; "
                      f"cache={results[name]['private_cache']!r}, "
                      f"public_cache={results[name]['public_cache']!r}", flush=True)
        finally:
            for process in processes:
                stop_server(process)
        assert results["fizzy"] == results["rizzy"], results


if __name__ == "__main__":
    main()
