#!/usr/bin/env python3
"""Verify Rizzy direct-upload IDs using pinned Fizzy's Rails verifiers."""

import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from sustained_mixed import SOURCE, prepare_rust, source_env, start_server, stop_server  # noqa: E402
from embed_parity import request, upload  # noqa: E402


def verify_with_fizzy(blob, env):
    script = '''require "json"
      blob = JSON.parse(STDIN.read)
      puts JSON.generate({
        signed_id: ActiveStorage::Blob.signed_id_verifier.verify(blob.fetch("signed_id"), purpose: "blob_id"),
        attachable_id: SignedGlobalID.parse(blob.fetch("attachable_sgid"), for: ActionText::Attachable::LOCATOR_NAME)&.model_id
      })'''
    result = subprocess.run(["bin/rails", "runner", script], cwd=SOURCE, env=env,
                            input=json.dumps(blob), check=True, capture_output=True, text=True)
    assert json.loads(result.stdout) == {"signed_id": blob["id"], "attachable_id": blob["id"]}


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet", "--bin", "rizzy"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-signed-blob-") as temporary:
        directory = Path(temporary)
        fizzy_env = source_env(directory, 2)
        rust_env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{directory / 'rizzy.sqlite3'}",
                    "RIZZY_STORAGE_DIR": str(directory / "files"),
                    "RIZZY_SECRET_KEY_BASE": fizzy_env["SECRET_KEY_BASE"]}
        rust_env, credentials = prepare_rust(directory, rust_env)
        server, url = start_server("rizzy", directory, rust_env, sorted(os.sched_getaffinity(0))[:2])
        target = {"name": "rizzy", "url": url, "account": str(credentials["account"]),
                  "token": credentials["token"]}
        try:
            blob = upload(target)
            verify_with_fizzy(blob, fizzy_env)
            status, _, board = request(target, "/boards", "POST", {"board": {"name": "Signed blob"}})
            assert status == 201
            markup = f'<p><action-text-attachment sgid="{blob["attachable_sgid"]}"></action-text-attachment></p>'
            status, _, card = request(target, f"/boards/{board['id']}/cards", "POST",
                                      {"card": {"title": "Signed embed", "description": markup}})
            assert status == 201
            assert card["has_attachments"] and blob["signed_id"] in card["description_html"]
            print("Rails accepted Rizzy blob signed ID and attachable SGID; the embedded file rendered")
        finally:
            stop_server(server)


if __name__ == "__main__":
    main()
