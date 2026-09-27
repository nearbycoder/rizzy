#!/usr/bin/env python3
"""A worker booting inside an export-cleanup minute waits for its next slot."""

import subprocess
import tempfile
import time
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path

from recurring_live_clock import BINARY, ROOT, check, port, setup, timezone_for


def main():
    subprocess.run(["cargo", "build", "--locked", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-startup-slot-") as directory:
        root = Path(directory)
        database, env, exports, imports = setup("exports", root)
        while True:
            current = datetime.now(timezone.utc)
            if 5 <= current.second <= 15:
                break
            time.sleep(.2)
        minute = current.replace(second=0, microsecond=0)
        address = f"127.0.0.1:{port()}"
        env = {**env, "TZ": timezone_for(minute, "exports"), "RIZZY_BIND": address}
        log = root / "server.log"
        with log.open("w") as output:
            process = subprocess.Popen([BINARY], cwd=ROOT, env=env,
                                       stdout=output, stderr=subprocess.STDOUT)
        try:
            for _ in range(100):
                try:
                    with urllib.request.urlopen(f"http://{address}/up", timeout=.2) as response:
                        assert response.status == 200
                    break
                except OSError:
                    time.sleep(.05)
            else:
                raise AssertionError(("server did not start", log.read_text()[-1000:]))
            assert datetime.now(timezone.utc) < minute + timedelta(minutes=1)
            slot = minute.strftime("%Y-%m-%dT%H")
            check("exports", database, exports, imports, slot, finished=False)
            until = minute + timedelta(minutes=1, seconds=2)
            while datetime.now(timezone.utc) < until:
                time.sleep(.2)
            check("exports", database, exports, imports, slot, finished=False)
            assert process.poll() is None, (process.returncode, log.read_text()[-1000:])
        finally:
            process.terminate()
            try:
                process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                process.kill()
                process.wait(timeout=5)
    print("Startup inside a scheduled minute did not run that expired export cleanup")


if __name__ == "__main__":
    main()
