#!/usr/bin/env python3
"""Exercise account-scoped email unsubscribe links without a session."""
import json
import os
import pathlib
import re
import socket
import sqlite3
import subprocess
import tempfile
import time
import urllib.error
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parents[1]
BINARY = ROOT / 'target/debug/rizzy'


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


opener = urllib.request.build_opener(NoRedirect())


def call(port, path, method='GET', data=None, token=None):
    headers = {'Accept': 'text/html'}
    if token:
        headers['Authorization'] = 'Bearer ' + token
        headers['Accept'] = 'application/json'
    raw = None
    if data is not None:
        headers['Content-Type'] = 'application/json'
        raw = json.dumps(data).encode()
    request = urllib.request.Request(f'http://127.0.0.1:{port}{path}', data=raw, method=method, headers=headers)
    try:
        response = opener.open(request, timeout=5)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def main():
    with tempfile.TemporaryDirectory() as folder:
        db_path = pathlib.Path(folder) / 'rizzy.sqlite3'
        env = {**os.environ, 'RIZZY_DATABASE_URL': f'sqlite://{db_path}', 'RIZZY_SEED_EMAIL': 'unsubscribe@example.test'}
        result = subprocess.run([BINARY, 'seed'], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r'Account: (\d+)', result.stdout).group(1)
        token = re.search(r'Bearer token: (\w+)', result.stdout).group(1)
        with socket.socket() as sock:
            sock.bind(('127.0.0.1', 0))
            port = sock.getsockname()[1]
        server = subprocess.Popen([BINARY], cwd=ROOT, env={**env, 'RIZZY_BIND': f'127.0.0.1:{port}'},
                                  stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
        try:
            for _ in range(70):
                try:
                    if call(port, '/up')[0] == 200:
                        break
                except OSError:
                    time.sleep(.05)
            else:
                raise RuntimeError('server did not start')
            with sqlite3.connect(db_path) as db:
                user_id = db.execute("select u.id from users u join identities i on i.id=u.identity_id where i.email_address='unsubscribe@example.test'").fetchone()[0]
            settings = f'/{account}/notifications/settings'
            status, _, _ = call(port, settings, 'PUT', {'user_settings': {'bundle_email_frequency': 'every_few_hours'}}, token)
            assert status == 204
            with sqlite3.connect(db_path) as db:
                link_token = db.execute('select token from notification_unsubscribe_tokens where user_id=?', (user_id,)).fetchone()[0]
                assert len(link_token) == 64
                db.execute("insert into notification_bundles(id,account_id,user_id,starts_at,ends_at,status,created_at,updated_at) select 'pending-test',account_id,id,datetime('now'),datetime('now','+4 hours'),0,datetime('now'),datetime('now') from users where id=?", (user_id,))
            path = f'/{account}/notifications/unsubscribe'
            status, headers, body = call(port, path + '/new?access_token=' + link_token)
            assert status == 200 and b'Unsubscribing' in body and b'id="unsubscribe"' in body
            status, headers, _ = call(port, path + '?access_token=bad', 'POST')
            assert status == 302 and headers['Location'] == f'/{account}/'
            status, headers, _ = call(port, path + '?access_token=' + link_token, 'POST')
            assert status == 302 and headers['Location'] == path + '?access_token=' + link_token
            with sqlite3.connect(db_path) as db:
                frequency = db.execute('select bundle_email_frequency from notification_settings where user_id=?', (user_id,)).fetchone()[0]
                bundles = db.execute('select count(*) from notification_bundles where user_id=?', (user_id,)).fetchone()[0]
            assert frequency == 'never' and bundles == 0
            status, _, body = call(port, path + '?access_token=' + link_token)
            assert status == 200 and b'unsubscribed' in body
            status, headers, _ = call(port, '/999/notifications/unsubscribe?access_token=' + link_token)
            assert status == 302 and headers['Location'] == '/999/'
            with sqlite3.connect(db_path) as db:
                db.execute("update notification_unsubscribe_tokens set expires_at='2000-01-01T00:00:00.000Z' where user_id=?", (user_id,))
            status, headers, _ = call(port, path + '?access_token=' + link_token)
            assert status == 302 and headers['Location'] == f'/{account}/'
            print('PASS unsubscribe valid, invalid, expired, cross-account, pending bundle cleanup')
        finally:
            server.terminate()
            server.wait(timeout=5)


if __name__ == '__main__':
    main()
