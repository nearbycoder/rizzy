#!/usr/bin/env python3
"""Compare push subscription persistence, validation and deletion with live Fizzy."""
import json
import os
import pathlib
import sqlite3
import time
import urllib.error
import urllib.request


class Target:
    def __init__(self, name, base, account, token, db):
        self.name, self.base, self.account, self.token, self.db = name, base, account, token, db

    def request(self, path, method='GET', body=None):
        headers = {'Accept': 'application/json', 'Authorization': 'Bearer ' + self.token}
        raw = None
        if body is not None:
            headers['Content-Type'] = 'application/json'
            raw = json.dumps(body).encode()
        request = urllib.request.Request(self.base + path, data=raw, headers=headers, method=method)
        try:
            response = urllib.request.urlopen(request, timeout=12)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            return response.status, response.read()

    def subscription_path(self):
        status, body = self.request(f'/{self.account}/users.json')
        assert status == 200, (self.name, status, body[:200])
        users = json.loads(body)
        user = next(user for user in users if user['role'] == 'owner')
        return f'/{self.account}/users/{user["id"]}/push_subscriptions', user['id']

    def record(self, endpoint):
        with sqlite3.connect(self.db) as conn:
            row = conn.execute('SELECT id,p256dh_key,auth_key,user_agent FROM push_subscriptions WHERE endpoint=?',
                               (endpoint,)).fetchone()
        return row


def base36(n):
    chars = '0123456789abcdefghijklmnopqrstuvwxyz'
    result = ''
    while n:
        n, remainder = divmod(n, 36)
        result = chars[remainder] + result
    return result.rjust(25, '0')


def main():
    source_creds = json.loads(pathlib.Path('/tmp/rizzy-fizzy-fresh-creds.json').read_text())
    rust_folder = pathlib.Path(pathlib.Path('/tmp/rizzy-settings-active-dir').read_text().strip())
    rust_creds = dict(line.split(': ', 1) for line in (rust_folder / 'seed.txt').read_text().splitlines())
    targets = [
        Target('Fizzy', os.environ.get('FIZZY_URL', 'http://127.0.0.1:3006'),
               source_creds['account'], source_creds['token'], 'upstream/storage/production.sqlite3'),
        Target('Rizzy', os.environ.get('RIZZY_URL', 'http://localhost:3038'),
               rust_creds['Account'], rust_creds['Bearer token'], str(rust_folder / 'rizzy.sqlite3')),
    ]
    paths = {target.name: target.subscription_path() for target in targets}
    endpoint = f'https://fcm.googleapis.com/fcm/send/rizzy-parity-{time.time_ns()}'
    payload = {'push_subscription': {'endpoint': endpoint, 'p256dh_key': '123', 'auth_key': '456'}}
    try:
        for target in targets:
            path, _ = paths[target.name]
            status, body = target.request(path + '.json', 'POST', payload)
            assert status == 201, (target.name, status, body[:250])
            row = target.record(endpoint)
            assert row and row[1:3] == ('123', '456'), (target.name, row)
            status, _ = target.request(path + '.json', 'POST', payload)
            assert status == 201, (target.name, status)
            invalid = {'push_subscription': {**payload['push_subscription'], 'endpoint': 'https://attacker.example.com/steal'}}
            status, _ = target.request(path + '.json', 'POST', invalid)
            assert status == 422, (target.name, status)
            print(target.name, 'create, duplicate, and forbidden endpoint matched')
    finally:
        for target in targets:
            row = target.record(endpoint)
            if row:
                record_id = base36(int.from_bytes(row[0], 'big')) if target.name == 'Fizzy' else row[0]
                path, _ = paths[target.name]
                status, body = target.request(path + '/' + record_id + '.json', 'DELETE')
                assert status == 204 and target.record(endpoint) is None, (target.name, status, body[:200])
    print('PASS push subscription parity')


if __name__ == '__main__':
    main()
