#!/usr/bin/env python3
"""Compare the rendered service worker and check Rizzy's PWA resources."""
import http.cookiejar
import json
import os
import re
import sqlite3
import urllib.error
import urllib.request

FIZZY_URL = os.environ.get('FIZZY_URL', 'http://127.0.0.1:3006')
RIZZY_URL = os.environ.get('RIZZY_URL', 'http://localhost:3038')
FIZZY_DB = os.environ.get('FIZZY_DB', 'upstream/storage/production.sqlite3')
RIZZY_EMAIL = os.environ.get('RIZZY_EMAIL', 'settings-verify@example.test')


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


class Target:
    def __init__(self, url, client_ip=None):
        self.url = url
        self.client_ip = client_ip
        self.opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())

    def request(self, path, method='GET', data=None):
        headers = {'Accept': 'application/json'}
        if self.client_ip:
            headers['X-Forwarded-For'] = self.client_ip
        if data is not None:
            headers['Content-Type'] = 'application/json'
            data = json.dumps(data).encode()
        req = urllib.request.Request(self.url + path, data=data, headers=headers, method=method)
        try:
            response = self.opener.open(req, timeout=10)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            return response.status, response.headers, response.read()

    def login(self, email, db=None):
        status, headers, body = self.request('/session', 'POST', {'email_address': email})
        assert status == 201, (status, body[:200])
        if db:
            with sqlite3.connect(db) as conn:
                code = conn.execute('SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id '
                                    'WHERE i.email_address=? ORDER BY m.created_at DESC LIMIT 1', (email,)).fetchone()[0]
        else:
            code = headers['X-Magic-Link-Code']
        status, _, body = self.request('/session/magic_link', 'POST', {'code': code})
        assert status == 200, (status, body[:200])


def main():
    fizzy = Target(FIZZY_URL, f'198.51.100.{10 + os.urandom(1)[0] % 230}')
    rizzy = Target(RIZZY_URL)
    status, _, _ = rizzy.request('/service-worker.js')
    assert status == 302
    fizzy.login(os.environ.get('FIZZY_EMAIL', 'bench@example.com'), FIZZY_DB)
    rizzy.login(RIZZY_EMAIL)
    status, _, source = fizzy.request('/service-worker.js')
    assert status == 200
    status, headers, rust = rizzy.request('/service-worker.js')
    assert status == 200 and headers.get_content_type() == 'text/javascript'
    normalized_source = re.sub(rb'importScripts\("[^\"]+turbo-offline-umd.min-[0-9a-f]+\.js"\)',
                               b'importScripts("/assets/pwa/turbo-offline-umd.min.js")', source)
    assert normalized_source == rust
    status, _, manifest = rizzy.request('/manifest.json')
    assert status == 200
    config = json.loads(manifest)
    assert config['name'] == 'Rizzy' and config['display'] == 'standalone' and config['scope'] == '/'
    for icon in config['icons']:
        status, headers, image = rizzy.request(icon['src'])
        assert status == 200 and headers.get_content_type() == 'image/png' and image.startswith(b'\x89PNG')
    for path in ('/assets/pwa/turbo-offline-umd.min.js', '/assets/pwa/bell.svg', '/assets/pwa/activity.svg'):
        status, _, body = rizzy.request(path)
        assert status == 200 and body
    print('PASS service-worker parity, authentication, manifest, offline bundle, icons')


if __name__ == '__main__':
    main()
