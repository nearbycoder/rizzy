#!/usr/bin/env python3
"""Check ActiveSupport timezone aliases on a running Rust board page."""
import http.cookiejar
import json
import tempfile
import urllib.request

from auth import stop
from draft_workflow_parity import NoRedirect, login, request
from search_html_parity import rust_target
from timezone_etag_parity import set_timezone
from platform_html_parity import cookie_for


def main():
    with tempfile.TemporaryDirectory() as temp:
        target, process = rust_target(temp)
        target["jar"] = http.cookiejar.CookieJar()
        target["opener"] = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(target["jar"]), NoRedirect())
        target["extra_headers"] = {}
        try:
            login(target)
            status, _, raw = request(target, f"/{target['account']}/boards", "POST",
                                     {"name": "Timezone alias"}, bearer=True)
            assert status == 201, (status, raw)
            board = json.loads(raw)
            path = f"/{target['account']}/boards/{board['id']}"
            try:
                validators = {}
                for zone in (None, "America/New_York", "Eastern Time (US & Canada)",
                             "Madrid", "Europe/Madrid", "Not/A_Time_Zone"):
                    set_timezone(target, zone)
                    status, headers, body = request(target, path, accept="text/html")
                    assert status == 200 and body, (zone, status)
                    validators[zone] = headers["ETag"]
                assert validators[None] == validators["Not/A_Time_Zone"]
                assert len({validators[None], validators["America/New_York"],
                            validators["Eastern Time (US & Canada)"], validators["Madrid"],
                            validators["Europe/Madrid"]}) == 5
                set_timezone(target, None)
                raw_alias = cookie_for(target, "timezone", "Eastern Time (US & Canada)")
                raw_alias.value = "Eastern Time (US & Canada)"
                target["jar"].set_cookie(raw_alias)
                status, headers, body = request(target, path, accept="text/html")
                assert status == 200 and body and headers["ETag"] == validators["Eastern Time (US & Canada)"]
                set_timezone(target, "Eastern Time (US & Canada)")
                target["extra_headers"]["If-None-Match"] = validators["Eastern Time (US & Canada)"]
                status, _, body = request(target, path, accept="text/html")
                assert status == 304 and not body, status
                set_timezone(target, "America/New_York")
                status, _, body = request(target, path, accept="text/html")
                assert status == 200 and body, status
            finally:
                set_timezone(target, None)
                target["extra_headers"].pop("If-None-Match", None)
                assert request(target, path, "DELETE", bearer=True)[0] == 204
        finally:
            stop(process)
    print("Rust timezone aliases matched the pinned source's ETag decisions")


if __name__ == "__main__":
    main()
