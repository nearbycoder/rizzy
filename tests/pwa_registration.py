#!/usr/bin/env python3
"""Authenticated full pages register the shared offline and push worker."""
import http.cookiejar
import json
import os
import pathlib
import re
import subprocess
import tempfile
import urllib.error
import urllib.request

from auth import BINARY, ROOT, start, stop


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


def main():
    with tempfile.TemporaryDirectory() as folder:
        database = pathlib.Path(folder) / 'rizzy.sqlite3'
        env = {**os.environ, 'RIZZY_DATABASE_URL': f'sqlite://{database}',
               'RIZZY_SEED_EMAIL': 'pwa-page@example.test', 'RIZZY_DEV_MAGIC_LINKS': '1'}
        seed = subprocess.run([BINARY, 'seed'], cwd=ROOT, env=env,
                              capture_output=True, text=True, check=True)
        account = re.search(r'Account: (\d+)', seed.stdout).group(1)
        port, server = start(env)
        opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())

        def call(path, method='GET', payload=None, extra_headers=None):
            headers = {'Accept': 'text/html', **(extra_headers or {})}
            if payload is not None:
                headers['Content-Type'] = 'application/json'
            req = urllib.request.Request(
                f'http://127.0.0.1:{port}{path}',
                data=json.dumps(payload).encode() if payload is not None else None,
                headers=headers, method=method)
            try:
                response = opener.open(req, timeout=5)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                return response.status, response.headers, response.read()

        try:
            status, _, anonymous = call('/session/new')
            assert status == 200 and b'serviceWorker.register' not in anonymous
            status, headers, _ = call('/session', 'POST', {'email_address': 'pwa-page@example.test'},
                                      {'Accept': 'application/json'})
            assert status == 201
            status, _, _ = call('/session/magic_link', 'POST',
                                {'code': headers['X-Magic-Link-Code']},
                                {'Accept': 'application/json'})
            assert status == 200
            for path in (f'/{account}/', f'/{account}/notifications/settings'):
                status, _, html = call(path)
                assert status == 200, (path, status)
                assert b'<link rel="manifest" href="/manifest.json">' in html, path
                assert b'navigator.serviceWorker.register' in html, path
                assert b"scope: '/'" in html, path
            status, headers, menu = call(f'/{account}/my/menu')
            assert status == 200 and b'navigator.serviceWorker.register' in menu
            etag = headers['ETag']
            status, _, cached = call(f'/{account}/my/menu', extra_headers={'If-None-Match': etag})
            assert status == 304 and cached == b''
            status, _, worker = call('/service-worker.js')
            assert status == 200 and b'TurboOffline.start()' in worker
            print('PASS authenticated PWA registration, anonymous exclusion, worker, menu ETag')
        finally:
            stop(server)


if __name__ == '__main__':
    main()
