#!/usr/bin/env python3
"""Exercise passkey registration, authentication, replay defense and management."""
import base64
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
import urllib.error
import urllib.parse
import urllib.request

SOURCE = os.environ.get('PASSKEY_TARGET') == 'fizzy'
BASE = os.environ.get('FIZZY_URL', 'http://127.0.0.1:3006') if SOURCE else os.environ.get('RIZZY_URL', 'http://localhost:3038')
DB = pathlib.Path(os.environ['FIZZY_DB'] if SOURCE else os.environ['RIZZY_DB'])
EMAIL = os.environ.get('FIZZY_EMAIL', 'bench@example.com') if SOURCE else os.environ.get('RIZZY_EMAIL', 'settings-verify@example.test')
RP_ID = urllib.parse.urlparse(BASE).hostname
ORIGIN = BASE
PASSKEYS_PATH = f"/{os.environ.get('FIZZY_ACCOUNT', '1')}/my/passkeys" if SOURCE else '/my/passkeys'
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


class NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, request, response, code, message, headers, new_url):
        return None


jar = http.cookiejar.CookieJar()
opener = urllib.request.build_opener(urllib.request.HTTPCookieProcessor(jar), NoRedirect())
csrf_token = None


def request(path, method='GET', data=None, accept='application/json'):
    headers = {'Accept': accept}
    if SOURCE and csrf_token:
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
    with sqlite3.connect(DB) as db:
        table = 'action_pack_passkeys' if SOURCE else 'passkeys'
        return db.execute(f'SELECT count(*) FROM {table} WHERE credential_id=?',
                          (b64(CREDENTIAL_ID),)).fetchone()[0]


def logout_source():
    global csrf_token
    if not SOURCE:
        return
    status, _, _ = request('/session', 'DELETE')
    assert status in (204, 302), status
    status, _, signin_html = request('/session/new', accept='text/html')
    assert status == 200
    match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', signin_html)
    assert match
    csrf_token = match.group(1).decode()


def main():
    global csrf_token
    login()
    status, _, html = request(PASSKEYS_PATH, accept='text/html')
    assert status == 200 and b'Register a passkey' in html, (status, html[:300])
    if not SOURCE:
        assert b'"residentKey":"required"' in html and b'"requireResidentKey":true' in html
    if SOURCE:
        match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', html)
        assert match
        csrf_token = match.group(1).decode()
    before = count()
    challenge_value = challenge('registration')
    status, headers, body = request(PASSKEYS_PATH, 'POST', registration(challenge_value))
    assert status == 302 and '/edit?created=true' in headers['Location'], (status, dict(headers), body[:300])
    assert count() == before + 1
    path = urllib.parse.urlparse(headers['Location']).path
    status, _, html = request(path, accept='text/html')
    assert status == 200 and b'Name your passkey' in html
    status, _, _ = request(path.removesuffix('/edit'), 'PATCH', {'passkey': {'name': 'Test key'}})
    assert status == 302
    status, _, html = request(PASSKEYS_PATH, accept='text/html')
    assert status == 200 and b'Test key' in html
    logout_source()
    with tempfile.TemporaryDirectory() as folder:
        der_path = pathlib.Path(folder) / 'key.der'
        key_path = pathlib.Path(folder) / 'key.pem'
        der_path.write_bytes(bytes.fromhex(KEY_HEX))
        subprocess.run(['openssl', 'pkey', '-inform', 'DER', '-in', str(der_path), '-out', str(key_path)],
                       capture_output=True, check=True)
        auth_challenge = challenge('authentication')
        status, _, body = request('/session/passkey.json', 'POST', assertion(auth_challenge, key_path))
        assert status == 200 and json.loads(body).get('session_token'), (status, body[:300])
        logout_source()
        status, _, body = request('/session/passkey.json', 'POST', assertion(auth_challenge, key_path))
        assert status == 401, (status, body[:300])
        wrong_challenge = challenge('authentication')
        status, _, body = request('/session/passkey.json', 'POST', assertion(wrong_challenge, key_path, 2, True))
        assert status == 401, (status, body[:300])
    if SOURCE:
        login()
        status, _, html = request(PASSKEYS_PATH, accept='text/html')
        assert status == 200
        match = re.search(rb'<meta name="csrf-token" content="([^"]+)"', html)
        assert match
        csrf_token = match.group(1).decode()
    status, _, _ = request(path.removesuffix('/edit'), 'DELETE')
    assert status == 302 and count() == before
    print('PASS passkey registration, management, signed login, replay, bad signature, deletion')


if __name__ == '__main__':
    main()
