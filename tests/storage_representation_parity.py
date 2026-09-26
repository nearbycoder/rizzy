#!/usr/bin/env python3
"""Compare generated image representations across source and Rust routes."""

import json
import os
import re
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
    prefix = f"/{target['account']}"
    status, _, raw = call(target, prefix + "/boards", "POST", {"board": {"name": "Representation parity"}})
    assert status == 201, (target["name"], status, raw[:150])
    board = json.loads(raw)
    try:
        blob = upload(target)
        markup = f'<action-text-attachment sgid="{blob["attachable_sgid"]}"></action-text-attachment>'
        status, _, raw = call(target, prefix + f"/boards/{board['id']}/cards", "POST",
                              {"card": {"title": "Representation", "description": markup}})
        assert status == 201, (target["name"], status, raw[:150])
        card = json.loads(raw)
        variant_times = None
        if target.get("storage"):
            directory = target["storage"] / "variants" / blob["key"]
            variants = list(directory.glob("*.jpg"))
            assert len(variants) == 2, (target["name"], variants)
            variant_times = {variant.name: variant.stat().st_mtime_ns for variant in variants}
        match = re.search(r'href="([^" ]*/representations/redirect/[^" ]+)"',
                          card["description_html"])
        assert match, (target["name"], card["description_html"])
        redirect = match.group(1)
        proxy = redirect.replace("/representations/redirect/", "/representations/proxy/")
        alias = redirect.replace("/representations/redirect/", "/representations/")
        for path in (redirect, alias):
            status, headers, _ = call(target, path)
            assert status == 302 and "/rails/active_storage/disk/" in headers["Location"], (
                target["name"], path, status, headers)
            status, final_headers, content = call(target, headers["Location"], bearer=False)
            assert status == 200 and content[:2] == b"\xff\xd8" and content != IMAGE, (
                target["name"], status, len(content))
            assert final_headers.get_content_type() == "image/jpeg"

        status, headers, content = call(target, proxy)
        assert status == 200 and content[:2] == b"\xff\xd8" and content != IMAGE, (
            target["name"], status, len(content))
        assert headers.get_content_type() == "image/jpeg"
        assert "public" not in headers["Cache-Control"]
        if variant_times is not None:
            assert {variant.name: variant.stat().st_mtime_ns for variant in variants} == variant_times
        private_cache = headers["Cache-Control"]

        variation = redirect.rsplit("/", 2)[1]
        tampered = redirect.replace("/" + variation + "/", "/" + variation[:-1] +
                                    ("a" if variation[-1] != "a" else "b") + "/")
        for path in (redirect, proxy, tampered):
            status, headers, _ = call(target, path, bearer=False)
            assert status == 302 and headers["Location"].endswith("/session/new"), (
                target["name"], path, status, headers)
        status, _, _ = call(target, tampered)
        assert status == 404, (target["name"], status)

        status, _, _ = call(target, prefix + f"/boards/{board['id']}/publication", "POST")
        assert status == 201, (target["name"], status)
        status, headers, public_content = call(target, proxy, bearer=False)
        assert status == 200 and public_content == content, (target["name"], status)
        assert "public" in headers["Cache-Control"]
        public_cache = headers["Cache-Control"]
        return {"redirect": 302, "alias": 302, "proxy": 200,
                "private": 302, "public": 200, "tampered": 404,
                "private_cache": private_cache, "public_cache": public_cache}
    finally:
        status, _, _ = call(target, prefix + f"/boards/{board['id']}", "DELETE")
        assert status == 204, (target["name"], status)
        if target.get("storage") and "blob" in locals():
            assert not (target["storage"] / "variants" / blob["key"]).exists()


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-representation-") as temporary:
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
                if name == "rizzy":
                    target["storage"] = directory / "storage"
                results[name] = check(target)
                print(f"{name}: representation redirect, alias, proxy, access, and tampering passed",
                      flush=True)
        finally:
            for process in processes:
                stop_server(process)
        assert results["fizzy"] == results["rizzy"], results


if __name__ == "__main__":
    main()
