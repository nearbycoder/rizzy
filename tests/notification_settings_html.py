#!/usr/bin/env python3
"""Exercise the browser notification preferences reached from bundle email."""
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

from auth import BINARY, ROOT, start, stop


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


def main():
    with tempfile.TemporaryDirectory() as folder:
        database = pathlib.Path(folder) / 'rizzy.sqlite3'
        env = {**os.environ, 'RIZZY_DATABASE_URL': f'sqlite://{database}',
               'RIZZY_SEED_EMAIL': 'settings-email@example.test', 'RIZZY_DEV_MAGIC_LINKS': '1',
               'RIZZY_DISABLE_PUSH_DELIVERY': '1',
               'RIZZY_VAPID_PRIVATE_KEY': 'IQ9Ur0ykXoHS9gzfYX0aBjy9lvdrjx_PFUXmie9YRcY'}
        seed = subprocess.run([BINARY, 'seed'], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r'Account: (\d+)', seed.stdout).group(1)
        token = re.search(r'Bearer token: (\w+)', seed.stdout).group(1)
        port, server = start(env)
        opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())

        def call(path, method='GET', body=None, accept='text/html', bearer=False):
            headers = {'Accept': accept}
            if bearer:
                headers['Authorization'] = 'Bearer ' + token
            if isinstance(body, dict):
                headers['Content-Type'] = 'application/json'
                body = json.dumps(body).encode()
            elif body is not None:
                headers['Content-Type'] = 'application/x-www-form-urlencoded'
            request = urllib.request.Request(f'http://127.0.0.1:{port}{path}', method=method, data=body, headers=headers)
            try:
                response = opener.open(request, timeout=5)
            except urllib.error.HTTPError as error:
                response = error
            with response:
                return response.status, response.headers, response.read()

        try:
            status, headers, _ = call('/session', 'POST', {'email_address': 'settings-email@example.test'}, 'application/json')
            assert status == 201
            status, _, _ = call('/session/magic_link', 'POST', {'code': headers['X-Magic-Link-Code']}, 'application/json')
            assert status == 200
            path = f'/{account}/notifications/settings'
            status, headers, _ = call(f'/{account}/boards', 'POST', {'board': {'name': 'Launch board'}},
                                      'application/json', True)
            assert status == 201
            board_id = re.search(r'/boards/([^/.]+)', headers['Location']).group(1)
            status, _, html = call(path)
            assert status == 200 and b'Notification Settings' in html and b'Email Notifications' in html
            assert b'id="global-container"' in html and b'id="menu-dialog"' in html
            assert b'class="header__title"' in html and b'class="settings__panel settings__panel--users panel shadow"' in html
            assert b'class="settings__scrollable-list flex flex-column gap-half"' in html
            assert b'data-controller="notifications"' in html and b'data-controller="form"' in html
            assert b'data-action="change->form#submit"' in html
            assert b'<button type="submit">Save</button>' not in html
            assert b'Boards' in html and b'value="every_few_hours" selected' in html
            assert b'Launch board' in html and b'data-board-involvement' in html
            assert b'class="button_to"' in html and b'name="_method" value="put"' in html
            assert b'event.preventDefault()' in html
            assert b'data-involvement="watching"' in html and b'aria-label="Stop watching"' in html
            assert b'fetch(button.dataset.url' in html
            involvement_path = f'/{account}/boards/{board_id}/involvement'
            status, _, _ = call(involvement_path, 'PUT', {'involvement': 'access_only'},
                                'application/json', True)
            assert status == 204
            status, _, html = call(path)
            assert status == 200 and b'data-involvement="access_only"' in html
            assert b'aria-label="Watch this"' in html
            status, _, _ = call(involvement_path, 'PUT', {'involvement': 'watching'},
                                'application/json', True)
            assert status == 204
            status, _, html = call(path)
            assert status == 200 and b'data-involvement="watching"' in html
            assert b'data-push-settings' in html and b'pushManager.subscribe' in html
            assert b'Turn ON push notifications on this device' in html
            form = urllib.parse.urlencode({'_method': 'patch', 'user_settings[bundle_email_frequency]': 'never'}).encode()
            status, headers, _ = call(path, 'POST', form)
            assert status == 302 and headers['Location'] == path
            status, _, body = call(path + '.json', accept='application/json', bearer=True)
            assert status == 200 and json.loads(body) == {'bundle_email_frequency': 'never'}
            form = urllib.parse.urlencode({'_method': 'patch', 'user_settings[bundle_email_frequency]': 'daily'}).encode()
            status, headers, _ = call(path, 'POST', form)
            assert status == 302
            status, _, html = call(path)
            assert b'value="daily" selected' in html
            with sqlite3.connect(database) as db:
                assert db.execute('select count(*) from notification_unsubscribe_tokens').fetchone()[0] == 1
            status, _, _ = call(path + '.json', 'PUT', {'user_settings': {'bundle_email_frequency': 'weekly'}}, 'application/json', True)
            assert status == 204
            status, _, html = call(path)
            assert b'value="weekly" selected' in html
            print('PASS notification settings browser view, board involvement, form, JSON, token issue')
        finally:
            stop(server)


if __name__ == '__main__':
    main()
