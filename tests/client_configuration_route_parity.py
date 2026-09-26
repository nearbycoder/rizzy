#!/usr/bin/env python3
"""Compare the parameterized mobile-configuration route with pinned Fizzy."""

import tempfile
import urllib.error
import urllib.request

from auth import stop
from search_html_parity import rust_target, source_target


CASES = (
    ("android_v1.json", "text/html"),
    ("ios_v1.json", "application/json"),
    ("android_v1", "application/json"),
    ("ios_v1", "application/json"),
    ("android_v1?format=json", "text/html"),
    ("android_v1", "text/html"),
    ("android_v1.html", "application/json"),
    ("android_v2000.json", "application/json"),
    ("ios_v2000.json", "application/json"),
    ("blackberry_v1.json", "application/json"),
    ("android_va.json", "application/json"),
    ("_v1.json", "application/json"),
)


def response(target, prefix, suffix, accept):
    request = urllib.request.Request(
        target["url"] + prefix + "/client_configurations/" + suffix,
        headers={"Accept": accept},
    )
    try:
        result = urllib.request.urlopen(request, timeout=10)
    except urllib.error.HTTPError as error:
        result = error
    with result:
        return (
            result.status,
            result.headers.get("Content-Type"),
            result.headers.get("Cache-Control"),
            result.headers.get("X-Robots-Tag"),
            result.read(),
        )


def main():
    source = source_target()
    with tempfile.TemporaryDirectory(prefix="rizzy-client-config-") as temp:
        rust, process = rust_target(temp)
        try:
            for use_account in (False, True):
                for suffix, accept in CASES:
                    expected = response(source, "/" + source["account"] if use_account else "", suffix, accept)
                    actual = response(rust, "/" + rust["account"] if use_account else "", suffix, accept)
                    assert actual == expected, (
                        use_account, suffix, accept, expected[:4], actual[:4],
                        expected[4][:100], actual[4][:100],
                    )
            print("Parameterized client configuration route matches Fizzy in 24 cases")
        finally:
            stop(process)


if __name__ == "__main__":
    main()
