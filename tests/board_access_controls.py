#!/usr/bin/env python3
"""Exercise board settings people controls and access changes with several users."""
import datetime
import http.cookiejar
import json
import os
import pathlib
import re
import sqlite3
import subprocess
import tempfile
import urllib.error
import urllib.parse
import urllib.request
import uuid

from auth import BINARY, ROOT, start, stop


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


def main():
    with tempfile.TemporaryDirectory() as folder:
        database = pathlib.Path(folder) / 'rizzy.sqlite3'
        env = {**os.environ, 'RIZZY_DATABASE_URL': f'sqlite://{database}',
               'RIZZY_SEED_EMAIL': 'board-access-owner@example.test', 'RIZZY_DEV_MAGIC_LINKS': '1'}
        seed = subprocess.run([BINARY, 'seed'], cwd=ROOT, env=env,
                              capture_output=True, text=True, check=True)
        account = re.search(r'Account: (\d+)', seed.stdout).group(1)
        token = re.search(r'Bearer token: (\w+)', seed.stdout).group(1)
        with sqlite3.connect(database) as db:
            account_id, owner_id = db.execute(
                "SELECT a.id,u.id FROM accounts a JOIN users u ON u.account_id=a.id "
                "WHERE a.external_account_id=? AND u.role='owner'", (account,)).fetchone()
            members = {}
            now = datetime.datetime.now(datetime.timezone.utc).isoformat()
            for name in ('Alice', 'Bob', 'Charlie'):
                identity_id, user_id = uuid.uuid4().hex, uuid.uuid4().hex
                db.execute('INSERT INTO identities(id,email_address,created_at) VALUES(?,?,?)',
                           (identity_id, f'{name.lower()}@example.test', now))
                db.execute("INSERT INTO users(id,account_id,identity_id,name,role,active,created_at) "
                           "VALUES(?,?,?,?, 'member',1,?)",
                           (user_id, account_id, identity_id, name, now))
                members[name] = user_id
        port, server = start(env)
        opener = urllib.request.build_opener(
            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())

        def call(path, method='GET', body=None, bearer=False, browser=None):
            headers = {'Accept': 'text/html'}
            if bearer:
                headers['Authorization'] = 'Bearer ' + token
                headers['Accept'] = 'application/json'
            if isinstance(body, dict):
                headers['Content-Type'] = 'application/json'
                body = json.dumps(body).encode()
            elif body is not None:
                headers['Content-Type'] = 'application/x-www-form-urlencoded'
            req = urllib.request.Request(f'http://127.0.0.1:{port}{path}',
                                         data=body, headers=headers, method=method)
            try:
                response = (browser or opener).open(req, timeout=5)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                return response.status, response.headers, response.read()

        try:
            status, headers, _ = call('/session', 'POST',
                                      {'email_address': 'board-access-owner@example.test'})
            assert status == 201
            status, _, _ = call('/session/magic_link', 'POST',
                                {'code': headers['X-Magic-Link-Code']})
            assert status == 200
            status, _, raw = call(f'/{account}/boards', 'POST',
                                  {'board': {'name': 'Restricted planning'}}, True)
            assert status == 201
            board_id = json.loads(raw)['id']
            path = f'/{account}/boards/{board_id}'
            status, _, _ = call(path, 'PATCH',
                                {'board': {'all_access': False},
                                 'user_ids': [owner_id, members['Alice']]}, True)
            assert status == 200
            status, _, html = call(path + '/edit')
            assert status == 200
            assert b'id="board-access-filter"' in html
            assert b'Select all' in html and b'Select none' in html
            controls = re.search(rb'<div id="board-access-controls"([^>]*)>', html)
            assert controls and b'hidden' not in controls.group(1)
            assert b'id="current-user-access"' in html
            positions = [html.index(name.encode()) for name in ('Alice', 'Bob', 'Charlie')]
            assert positions[0] < positions[1] < positions[2]
            for name in ('Alice', 'Bob', 'Charlie'):
                tag = next(tag for tag in re.findall(rb'<input\b[^>]*>', html)
                           if f'value="{members[name]}"'.encode() in tag)
                assert (b'checked' in tag) == (name == 'Alice'), (name, tag)
            form = urllib.parse.urlencode([
                ('_method', 'patch'), ('board[name]', 'Restricted planning'),
                ('board[all_access]', '0'), ('user_ids[]', owner_id),
                ('user_ids[]', members['Bob']),
            ]).encode()
            status, headers, _ = call(path, 'POST', form)
            assert status == 302 and headers['Location'] == path + '/edit'
            with sqlite3.connect(database) as db:
                users = {row[0] for row in db.execute(
                    'SELECT user_id FROM accesses WHERE board_id=?', (board_id,))}
            assert users == {owner_id, members['Bob']}, users
            status, _, html = call(path + '/edit')
            assert status == 200 and b'Bob' in html and b'Alice' in html
            bob_browser = urllib.request.build_opener(
                urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
            status, headers, _ = call('/session', 'POST',
                                      {'email_address': 'bob@example.test'}, browser=bob_browser)
            assert status == 201
            status, _, _ = call('/session/magic_link', 'POST',
                                {'code': headers['X-Magic-Link-Code']}, browser=bob_browser)
            assert status == 200
            status, _, member_html = call(path + '/edit', browser=bob_browser)
            assert status == 200
            assert re.search(rb'<button[^>]*id="access-select-all"[^>]*disabled', member_html)
            assert re.search(rb'<button[^>]*id="access-select-none"[^>]*disabled', member_html)
            assert all(b'disabled' in tag for tag in re.findall(rb'<input\b[^>]*name="user_ids\[\]"[^>]*>', member_html))
            print('PASS board access settings controls, selected order, save and revocation')
        finally:
            stop(server)


if __name__ == '__main__':
    main()
