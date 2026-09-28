#!/usr/bin/env python3
"""Check Fizzy-compatible step Turbo actions and JSON state on a fresh Rizzy DB."""

import http.cookiejar
import os
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
from card_workflow import expect  # noqa: E402
from draft_workflow_parity import NoRedirect, login  # noqa: E402
from sustained_mixed import prepare_rust, start_server, stop_server  # noqa: E402


def turbo(target, path, method, data=b""):
    request = urllib.request.Request(
        target["url"] + f"/{target['account']}" + path, data=data, method=method,
        headers={"Authorization": "Bearer " + target["token"],
                 "Accept": "text/vnd.turbo-stream.html",
                 "Content-Type": "application/x-www-form-urlencoded"},
    )
    with target["opener"].open(request, timeout=10) as response:
        return response.status, response.headers["Content-Type"], response.read().decode()


def main():
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-step-turbo-") as temp:
        directory = Path(temp)
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{directory / 'db.sqlite3'}",
               "RIZZY_STORAGE_DIR": str(directory / "storage"),
               "RIZZY_DEV_MAGIC_LINKS": "1",
               "RIZZY_DISABLE_WEBHOOK_DELIVERY": "1",
               "RIZZY_DISABLE_PUSH_DELIVERY": "1"}
        env, credentials = prepare_rust(directory, env)
        process, url = start_server("rizzy", directory, env,
                                    sorted(os.sched_getaffinity(0))[:4])
        target = {"url": url, "account": str(credentials["account"]),
                  "token": credentials["token"], "name": "rizzy",
                  "email": "owner@example.com",
                  "opener": urllib.request.build_opener(
                      urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                      NoRedirect())}
        try:
            login(target)
            board, _ = expect(target, "/boards", "POST", {"board": {"name": "Step Turbo"}}, 201)
            card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                             {"card": {"title": "Step stream"}}, 201)
            number = card["number"]
            card_id = card["id"]
            path = f"/cards/{number}/steps"
            status, content_type, stream = turbo(
                target, path, "POST", b"step%5Bcontent%5D=Turbo+step&step%5Bcompleted%5D=1")
            assert status == 200 and content_type.startswith("text/vnd.turbo-stream.html")
            assert f'action="before" target="new_step_card_{card_id}"' in stream, stream
            steps, _ = expect(target, path)
            assert len(steps) == 1 and steps[0]["content"] == "Turbo step"
            assert steps[0]["completed"] is True
            step_id = steps[0]["id"]
            assert f'id="step_{step_id}"' in stream

            status, content_type, stream = turbo(
                target, f"{path}/{step_id}", "PUT",
                b"step%5Bcontent%5D=Changed&step%5Bcompleted%5D=0")
            assert status == 200 and content_type.startswith("text/vnd.turbo-stream.html")
            assert f'action="replace" target="step_{step_id}"' in stream, stream
            assert f'action="replace" target="bubble_card_{card_id}"' in stream, stream
            step, _ = expect(target, f"{path}/{step_id}")
            assert step == {"id": step_id, "content": "Changed", "completed": False}

            status, content_type, stream = turbo(target, f"{path}/{step_id}", "DELETE")
            assert status == 200 and content_type.startswith("text/vnd.turbo-stream.html")
            assert f'action="remove" target="step_{step_id}"' in stream, stream
            assert expect(target, path)[0] == []

            created, headers = expect(target, path, "POST",
                                      {"step": {"content": "JSON step", "completed": True}}, 201)
            assert created["completed"] is True
            assert headers["Location"] == f"/{target['account']}{path}/{created['id']}.json"
            updated, _ = expect(target, f"{path}/{created['id']}", "PUT",
                                {"step": {"completed": False}})
            assert updated["completed"] is False
            expect(target, f"{path}/{created['id']}", "DELETE", status=204)
            print("Step Turbo and JSON create/update/delete passed")
        finally:
            stop_server(process)


if __name__ == "__main__":
    main()
