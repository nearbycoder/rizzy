#!/usr/bin/env python3
"""Compare the pinned Rails modern-browser gate with Rizzy."""

import json
import os
import time
import urllib.error
import urllib.parse
import urllib.request


AGENTS = {
    "chrome_118": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/118.0.0.0 Safari/537.36",
    "chrome_120": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "firefox_120": "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:120.0) Gecko/20100101 Firefox/120.0",
    "firefox_121": "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:121.0) Gecko/20100101 Firefox/121.0",
    "safari_17_1": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.1 Safari/605.1.15",
    "safari_17_2": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.2 Safari/605.1.15",
    "opera_105": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/119.0.0.0 Safari/537.36 OPR/105.0.0.0",
    "opera_106": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 OPR/106.0.0.0",
    "ie_11": "Mozilla/5.0 (Windows NT 6.3; Trident/7.0; rv:11.0) like Gecko",
    "old_edge": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/118.0.0.0 Safari/537.36 Edg/118.0.2088.76",
    "google_image_proxy": "Mozilla/5.0 (Windows NT 5.1; rv:11.0) Gecko Firefox/11.0 (via ggpht.com GoogleImageProxy)",
    "fake_bot": "TotallyFakeBot/1.0 (NonsenseBrowser; Testing)",
    "baidu": "Mozilla/5.0 (Linux; Android 7.0; HUAWEI NXT-AL10 Build/HUAWEINXT-AL10; wv) AppleWebKit/537.36 (KHTML, like Gecko) Version/4.0 Chrome/48.0.2564.116 Mobile Safari/537.36 baidubrowser/7.9.12.0 (Baidu; P1 7.0)NULL",
    "opera_legacy_105": "Opera/9.80 (Windows NT 10.0; U; en) Presto/2.12.388 Version/105.0",
    "opera_legacy_106": "Opera/9.80 (Windows NT 10.0; U; en) Presto/2.12.388 Version/106.0",
    "ios_safari_17_1": "Mozilla/5.0 (iPhone; CPU iPhone OS 17_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Safari/604.1",
    "ios_safari_17_2": "Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Mobile/15E148 Safari/604.1",
    "ios_chrome_118": "Mozilla/5.0 (iPhone; CPU iPhone OS 17_1 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/118.0.0.0 Mobile/15E148 Safari/604.1",
    "ios_chrome_120": "Mozilla/5.0 (iPhone; CPU iPhone OS 17_2 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) CriOS/120.0.0.0 Mobile/15E148 Safari/604.1",
}


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args):
        return None


OPENER = urllib.request.build_opener(NoRedirect())


def request(target, path, agent, token=None, method="GET", payload=None):
    headers = {"User-Agent": agent, "Accept": "application/json" if token else "text/html"}
    if token:
        headers["Authorization"] = "Bearer " + token
    if payload is not None:
        headers["Content-Type"] = "application/json"
    try:
        response = OPENER.open(urllib.request.Request(
            target["url"] + path, headers=headers, method=method,
            data=json.dumps(payload).encode() if payload is not None else None,
        ), timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def run(target):
    blocked = {"chrome_118", "firefox_120", "safari_17_1", "opera_105", "ie_11",
               "opera_legacy_105", "ios_safari_17_1", "ios_chrome_118"}
    results = {}
    for name, agent in AGENTS.items():
        status, headers, body = request(target, "/session/new", agent)
        expected = 406 if name in blocked else 200
        assert status == expected, (target["name"], name, status)
        if expected == 406:
            assert body == open("upstream/public/406-unsupported-browser.html", "rb").read()
            assert headers.get("X-Robots-Tag") is None
            assert headers.get("Vary") == "Accept"
            assert headers.get("Cache-Control") == "no-cache"
        else:
            assert headers.get("X-Robots-Tag") == "none"
        results[name] = status

    old = AGENTS["chrome_118"]
    boards = "/" + target["account"] + "/boards"
    for name, path, token, expected, redirect_to in (
        ("private_with_token", boards, target["token"], 406, None),
        ("private_without_token", boards, None, 302, "/session/new"),
        ("worker_without_token", "/service-worker", None, 302, "/session/new"),
        ("signup_redirect", "/signup", None, 301, "/signup/new"),
        ("signup_completion_without_token", "/signup/completion/new", None, 302, "/session/new"),
        ("health", "/up", None, 200, None),
        ("static_css", "/error.css", None, 200, None),
    ):
        status, headers, body = request(target, path, old, token)
        assert status == expected, (target["name"], name, status, body[:120])
        if status == 406:
            assert body == open("upstream/public/406-unsupported-browser.html", "rb").read()
        if redirect_to:
            assert urllib.parse.urlparse(headers["Location"]).path == redirect_to
            assert headers.get("X-Robots-Tag") is None
            assert headers.get("Vary") is None
        results[name] = status
    status, _, before = request(target, boards, AGENTS["chrome_120"], target["token"])
    assert status == 200
    status, _, body = request(
        target, boards, old, target["token"], method="POST",
        payload={"board": {"name": f"Blocked old browser {time.time_ns()}"}},
    )
    assert status == 406 and body == open("upstream/public/406-unsupported-browser.html", "rb").read()
    status, _, after = request(target, boards, AGENTS["chrome_120"], target["token"])
    assert status == 200 and before == after, (target["name"], "blocked_write_modified_boards")
    results["blocked_write"] = 406
    return results


def main():
    targets = [
        {"name": name.lower(), "url": os.environ[f"{name}_URL"].rstrip("/"),
         "account": os.environ[f"{name}_ACCOUNT"], "token": os.environ[f"{name}_TOKEN"]}
        for name in ("FIZZY", "RIZZY")
    ]
    source, rust = (run(target) for target in targets)
    assert source == rust, (source, rust)
    print(f"Modern-browser decisions matched on {len(source)} user-agent and route cases")


if __name__ == "__main__":
    main()
