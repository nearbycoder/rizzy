#!/usr/bin/env python3
"""Exercise passkey registration, authentication, replay defense and management."""
import base64
import concurrent.futures
import hashlib
import http.cookiejar
import json
import os
import pathlib
import re
import sqlite3
import struct
import subprocess
import tempfile
import threading
import urllib.error
import urllib.parse
import urllib.request

SOURCE = os.environ.get('PASSKEY_TARGET') == 'fizzy'
BASE = os.environ.get('FIZZY_URL', 'http://127.0.0.1:3006') if SOURCE else os.environ.get('RIZZY_URL', 'http://localhost:3038')
MYSQL_URL = os.environ.get('RIZZY_TEST_MYSQL_URL') if not SOURCE else None
SOURCE_MYSQL_DATABASE = os.environ.get('FIZZY_MYSQL_DATABASE') if SOURCE else None
DB = None if MYSQL_URL or SOURCE_MYSQL_DATABASE else pathlib.Path(
    os.environ['FIZZY_DB'] if SOURCE else os.environ['RIZZY_DB'])
EMAIL = os.environ.get('FIZZY_EMAIL', 'bench@example.com') if SOURCE else os.environ.get('RIZZY_EMAIL', 'settings-verify@example.test')
RP_ID = urllib.parse.urlparse(BASE).hostname
ORIGIN = BASE
PASSKEYS_PATH = (f"/{os.environ.get('FIZZY_ACCOUNT', '1')}/my/passkeys" if SOURCE
                 else f"/{os.environ.get('RIZZY_ACCOUNT', '1')}/my/passkeys")
KEY_HEX = ('307702010104201dd589de7210b3318620f32150e3012cce021519df1d6e9e01'
           '0471146d395cdca00a06082a8648ce3d030107a14403420004116847fe19e1ad'
           '4471ab9980d7ff9cc1e4c7cb7a3af00e082b64fcd84f5ae70114c2495eef16f'
           '542b5e57dd1b263661624e3cf28f581b57a441edbd756a41d0e')
COSE_HEX = ('a5010203262001215820116847fe19e1ad4471ab9980d7ff9cc1'
            'e4c7cb7a3af00e082b64fcd84f5ae70122582014c2495eef16f542b5e57dd1b2'
            '63661624e3cf28f581b57a441edbd756a41d0e')
CBOR_PREFIX = bytes.fromhex('a363666d74646e6f6e656761747453746d74a068617574684461746158a4')
CREDENTIAL_ID = os.urandom(32)


def b64(data):
    return base64.urlsafe_b64encode(data).decode().rstrip('=')


def source_mysql_scalar(statement):
    client = os.environ.get('FIZZY_MYSQL_CLIENT', '/tmp/rizzy-mysql8/bin/mysql')
    env = {**os.environ, 'MYSQL_PWD': os.environ.get('FIZZY_MYSQL_PASSWORD', 'rizzy'),
           'LD_LIBRARY_PATH': str(pathlib.Path(client).resolve().parents[1] / 'lib')}
    result = subprocess.run(
        [client, '--no-defaults', '--batch', '--skip-column-names',
         '--host=' + os.environ.get('FIZZY_MYSQL_HOST', '127.0.0.1'),
         '--port=' + os.environ.get('FIZZY_MYSQL_PORT', '33080'),
         '--user=' + os.environ.get('FIZZY_MYSQL_USER', 'rizzy'),
         '--database=' + SOURCE_MYSQL_DATABASE, '--execute=' + statement],
        env=env, capture_output=True, text=True, check=True)
    return result.stdout.strip()


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


jar = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar), NoRedirect())
csrf_token = None


def request(path, method='GET', data=None, accept='application/json'):
    headers = {'Accept': accept}
    if csrf_token:
        headers['X-CSRF-Token'] = csrf_token
    if isinstance(data, dict):
        raw = json.dumps(data, separators=(',', ':')).encode()
        headers['Content-Type'] = 'application/json'
    elif data is not None:
        raw = data
        headers['Content-Type'] = 'application/x-www-form-urlencoded'
    else:
        raw = None
    req = urllib.request.Request(BASE + path, data=raw, headers=headers, method=method)
    try:
        response = opener.open(req, timeout=15)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        return response.status, response.headers, response.read()


def client_data(challenge, kind):
    return json.dumps({'challenge': challenge, 'origin': ORIGIN, 'type': kind}, separators=(',', ':'))


def challenge(purpose):
    status, _, body = request('/my/passkey_challenge', 'POST', {'purpose': purpose})
    assert status == 200, (status, body[:300])
    return json.loads(body)['challenge']


def login():
    status, headers, body = request('/session', 'POST', {'email_address': EMAIL})
    assert status == 201, (status, body[:300])
    if SOURCE:
        if SOURCE_MYSQL_DATABASE:
            safe_email = EMAIL.replace("'", "''")
            code = source_mysql_scalar(
                "SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id "
                f"WHERE i.email_address='{safe_email}' ORDER BY m.created_at DESC LIMIT 1")
        else:
            with sqlite3.connect(DB) as db:
                code = db.execute('SELECT m.code FROM magic_links m JOIN identities i ON i.id=m.identity_id '
                                  'WHERE i.email_address=? ORDER BY m.created_at DESC LIMIT 1',
                                  (EMAIL,)).fetchone()[0]
    else:
        code = headers.get('X-Magic-Link-Code')
    assert code
    status, _, body = request('/session/magic_link', 'POST', {'code': code})
    assert status == 200, (status, body[:300])


def registration(challenge_value):
    data = (hashlib.sha256(RP_ID.encode()).digest() + bytes([0x45]) + struct.pack('>I', 0)
            + bytes(16) + struct.pack('>H', len(CREDENTIAL_ID)) + CREDENTIAL_ID
            + bytes.fromhex(COSE_HEX))
    assert len(data) == 164
    attestation = b64(CBOR_PREFIX + data)
    return {'passkey': {'client_data_json': client_data(challenge_value, 'webauthn.create'),
                        'attestation_object': attestation, 'transports': ['internal']}}


def assertion(challenge_value, key_path, counter=1, alter_signature=False):
    client = client_data(challenge_value, 'webauthn.get')
    auth_data = hashlib.sha256(RP_ID.encode()).digest() + bytes([0x05]) + struct.pack('>I', counter)
    signed = auth_data + hashlib.sha256(client.encode()).digest()
    signature = subprocess.run(['openssl', 'dgst', '-sha256', '-sign', str(key_path)],
                               input=signed, capture_output=True, check=True).stdout
    if alter_signature:
        signature = signature[:-1] + bytes([signature[-1] ^ 1])
    return {'passkey': {'id': b64(CREDENTIAL_ID), 'client_data_json': client,
                        'authenticator_data': b64(auth_data), 'signature': b64(signature)}}


def count():
    if SOURCE_MYSQL_DATABASE:
        return int(source_mysql_scalar(
            f"SELECT COUNT(*) FROM action_pack_passkeys WHERE credential_id='{b64(CREDENTIAL_ID)}'"))
    if MYSQL_URL:
        from mysql_core import mysql_scalar
        return int(mysql_scalar(MYSQL_URL, f"SELECT COUNT(*) FROM passkeys WHERE credential_id='{b64(CREDENTIAL_ID)}'"))
    with sqlite3.connect(DB) as db:
        table = 'action_pack_passkeys' if SOURCE else 'passkeys'
        return db.execute(f'SELECT count(*) FROM {table} WHERE credential_id=?',
                          (b64(CREDENTIAL_ID),)).fetchone()[0]


def logout():
    global csrf_token
    status, _, _ = request('/session', 'DELETE')
    assert status in (204, 302), status
    status, _, signin_html = request('/session/new', accept='text/html')
    assert status == 200
    if not SOURCE:
        assert b'X-CSRF-Token' in signin_html
    match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', signin_html)
    assert match
    csrf_token = match.group(1).decode()


def main():
    global csrf_token
    redirects = {}
    login()
    status, _, html = request(PASSKEYS_PATH, accept='text/html')
    assert status == 200 and b'Register a passkey' in html, (status, html[:300])
    if not SOURCE:
        assert b'"residentKey":"required"' in html and b'"requireResidentKey":true' in html
        assert b'X-CSRF-Token' in html
    match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', html)
    assert match
    csrf_token = match.group(1).decode()
    before = count()
    challenge_value = challenge('registration')
    status, headers, body = request(PASSKEYS_PATH, 'POST', registration(challenge_value))
    assert status == 302 and '/edit?created=true' in headers['Location'], (status, dict(headers), body[:300])
    redirects['create'] = (status, headers.get_content_type())
    assert count() == before + 1
    edit_url = urllib.parse.urlparse(headers['Location'])
    path = edit_url.path
    assert path.startswith(PASSKEYS_PATH + '/') and path.endswith('/edit'), path
    status, _, html = request(path + '?' + edit_url.query, accept='text/html')
    assert status == 200 and b'Name your passkey' in html
    assert b'Your passkey has been registered.' in html
    assert (PASSKEYS_PATH + '/' + path.split('/')[-2]).encode() in html
    status, headers, _ = request(path.removesuffix('/edit'), 'PATCH', {'passkey': {'name': 'Test key'}})
    assert status == 302 and urllib.parse.urlparse(headers['Location']).path == PASSKEYS_PATH
    redirects['update'] = (status, headers.get_content_type())
    status, _, html = request(PASSKEYS_PATH, accept='text/html')
    assert status == 200 and b'Test key' in html
    logout()
    with tempfile.TemporaryDirectory() as folder:
        der_path = pathlib.Path(folder) / 'key.der'
        key_path = pathlib.Path(folder) / 'key.pem'
        der_path.write_bytes(bytes.fromhex(KEY_HEX))
        subprocess.run(['openssl', 'pkey', '-inform', 'DER', '-in', str(der_path), '-out', str(key_path)],
                       capture_output=True, check=True)
        auth_challenge = challenge('authentication')
        status, _, body = request('/session/passkey.json', 'POST', assertion(auth_challenge, key_path))
        assert status == 200 and json.loads(body).get('session_token'), (status, body[:300])
        logout()
        status, _, body = request('/session/passkey.json', 'POST', assertion(auth_challenge, key_path))
        assert status == 401, (status, body[:300])
        wrong_challenge = challenge('authentication')
        status, _, body = request('/session/passkey.json', 'POST', assertion(wrong_challenge, key_path, 2, True))
        assert status == 401, (status, body[:300])
        if os.environ.get('PASSKEY_RACE') == '1':
            race_challenge = challenge('authentication')
            credential = assertion(race_challenge, key_path, 2)
            barrier = threading.Barrier(2)

            def compete():
                barrier.wait()
                return request('/session/passkey.json', 'POST', credential)[0]

            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                statuses = sorted(pool.map(lambda _: compete(), range(2)))
            assert statuses == [200, 401], statuses
    login()
    status, _, html = request(PASSKEYS_PATH, accept='text/html')
    assert status == 200
    match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', html)
    assert match
    csrf_token = match.group(1).decode()
    status, headers, _ = request(path.removesuffix('/edit'), 'DELETE')
    assert status == 302 and urllib.parse.urlparse(headers['Location']).path == PASSKEYS_PATH
    redirects['destroy'] = (status, headers.get_content_type())
    assert count() == before
    print('PASS passkey registration, management, signed login, replay, bad signature, deletion')
    print(json.dumps(redirects, sort_keys=True))


if __name__ == '__main__':
    main()
