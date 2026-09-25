#!/usr/bin/env python3
"""Generate a notification through the API and deliver its due bundle by sendmail."""
import hashlib
import os
import pathlib
import re
import sqlite3
import subprocess
import tempfile
import time
import urllib.request

from auth import BINARY, ROOT, start, stop
from join import call


def main():
    with tempfile.TemporaryDirectory() as folder:
        folder = pathlib.Path(folder)
        database = folder / 'rizzy.sqlite3'
        mailbox = folder / 'mailbox.txt'
        sendmail = folder / 'sendmail'
        sendmail.write_text(f"#!/bin/sh\ncat >> '{mailbox}'\n")
        sendmail.chmod(0o700)
        env = {**os.environ, 'RIZZY_DATABASE_URL': f'sqlite://{database}',
               'RIZZY_SENDMAIL': str(sendmail), 'RIZZY_MAIL_FROM': 'rizzy@example.test',
               'RIZZY_PUBLIC_URL': 'http://localhost:3038', 'RIZZY_DISABLE_WEBHOOK_DELIVERY': '1'}
        seed = subprocess.run([BINARY, 'seed'], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r'Account: (\d+)', seed.stdout).group(1)
        owner_token = re.search(r'Bearer token: (\w+)', seed.stdout).group(1)
        port, server = start(env)
        try:
            board, _ = call(port, f'/{account}/boards', 'POST', {'board': {'name': 'Mail board', 'all_access': False}}, bearer=owner_token, status=201)
            board_id = board['id']
            owner_id = board['creator']['id']
            with sqlite3.connect(database) as db:
                account_id = db.execute('select account_id from boards where id=?', (board_id,)).fetchone()[0]
                db.execute("insert into identities(id,email_address,created_at) values('mail-identity','mail-recipient@example.test',datetime('now'))")
                db.execute("insert into users(id,account_id,identity_id,name,role,verified_at,created_at) values('mail-user',?,'mail-identity','Mail Recipient','member',datetime('now'),datetime('now'))", (account_id,))
                db.execute("insert into access_tokens(id,identity_id,token_hash,description,permission,created_at) values('mail-token','mail-identity',?,'Test','write',datetime('now'))", (hashlib.sha256(b'mail-token').hexdigest(),))
            call(port, f'/{account}/boards/{board_id}', 'PUT', {'user_ids': [owner_id, 'mail-user']}, bearer=owner_token)
            call(port, f'/{account}/boards/{board_id}/involvement', 'PUT', {'involvement': 'watching'}, bearer='mail-token', status=204)
            card, _ = call(port, f'/{account}/boards/{board_id}/cards', 'POST', {'card': {'title': 'Bundle card'}}, bearer=owner_token, status=201)
            notices, _ = call(port, f'/{account}/notifications', bearer='mail-token')
            assert len(notices) == 1
            with sqlite3.connect(database) as db:
                bundles = db.execute("select id,starts_at,ends_at from notification_bundles where user_id='mail-user' and status=0").fetchall()
                assert len(bundles) == 1, bundles
                start_at, end_at = bundles[0][1:]
                from datetime import datetime
                assert 14390 <= (datetime.fromisoformat(end_at) - datetime.fromisoformat(start_at)).total_seconds() <= 14410
            call(port, f'/{account}/cards/{card["number"]}/closure', 'POST', bearer=owner_token, status=204)
            time.sleep(2)
            with sqlite3.connect(database) as db:
                assert db.execute("select count(*) from notification_bundles where user_id='mail-user'").fetchone()[0] == 1
                from datetime import datetime, timezone, timedelta
                cutoff = (datetime.now(timezone.utc) - timedelta(seconds=1)).isoformat(timespec='milliseconds').replace('+00:00', 'Z')
                db.execute("update notification_bundles set ends_at=? where user_id='mail-user'", (cutoff,))
            run = subprocess.run([BINARY, 'deliver-notifications'], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
            assert 'Delivered 1 notification bundles' in run.stdout, (run.stdout, run.stderr)
            mail = mailbox.read_text()
            assert 'To: mail-recipient@example.test' in mail and 'Subject: Rizzy: New notifications' in mail
            assert 'Bundle card' in mail and 'Mail board' in mail and 'List-Unsubscribe:' in mail
            with sqlite3.connect(database) as db:
                assert db.execute("select status from notification_bundles where user_id='mail-user'").fetchone()[0] == 2
                link_token = db.execute("select token from notification_unsubscribe_tokens where user_id='mail-user'").fetchone()[0]
            assert f'access_token={link_token}' in mail
            url = f'http://127.0.0.1:{port}/{account}/notifications/unsubscribe?access_token={link_token}'
            opener = urllib.request.build_opener(urllib.request.HTTPRedirectHandler())
            # Redirect target is inside the same local test server.
            with opener.open(urllib.request.Request(url, method='POST'), timeout=5) as response:
                assert response.status == 200 and b'unsubscribed' in response.read()
            with sqlite3.connect(database) as db:
                assert db.execute("select bundle_email_frequency from notification_settings where user_id='mail-user'").fetchone()[0] == 'never'
            call(port, f'/{account}/cards/{card["number"]}/closure', 'DELETE', bearer=owner_token, status=204)
            with sqlite3.connect(database) as db:
                assert db.execute("select count(*) from notification_bundles where user_id='mail-user' and status=0").fetchone()[0] == 0
            settings = f'/{account}/notifications/settings'
            call(port, settings, 'PUT', {'user_settings': {'bundle_email_frequency': 'daily'}}, bearer='mail-token', status=204)
            call(port, f'/{account}/boards/{board_id}/cards', 'POST', {'card': {'title': 'Daily card'}}, bearer=owner_token, status=201)
            with sqlite3.connect(database) as db:
                start_at, end_at = db.execute("select starts_at,ends_at from notification_bundles where user_id='mail-user' and status=0 order by created_at desc limit 1").fetchone()
                assert 86390 <= (datetime.fromisoformat(end_at) - datetime.fromisoformat(start_at)).total_seconds() <= 86410
            call(port, settings, 'PUT', {'user_settings': {'bundle_email_frequency': 'weekly'}}, bearer='mail-token', status=204)
            call(port, f'/{account}/boards/{board_id}/cards', 'POST', {'card': {'title': 'Weekly card'}}, bearer=owner_token, status=201)
            with sqlite3.connect(database) as db:
                start_at, end_at = db.execute("select starts_at,ends_at from notification_bundles where user_id='mail-user' and status=0 order by created_at desc limit 1").fetchone()
                assert 604790 <= (datetime.fromisoformat(end_at) - datetime.fromisoformat(start_at)).total_seconds() <= 604810
            time.sleep(2)
            cutoff = (datetime.now(timezone.utc) - timedelta(seconds=1)).isoformat(timespec='milliseconds').replace('+00:00', 'Z')
            with sqlite3.connect(database) as db:
                db.execute("update notification_bundles set ends_at=? where user_id='mail-user' and status=0", (cutoff,))
            failing = folder / 'failing-sendmail'
            failing.write_text('#!/bin/sh\nexit 1\n')
            failing.chmod(0o700)
            result = subprocess.run([BINARY, 'deliver-notifications'], cwd=ROOT,
                                    env={**env, 'RIZZY_SENDMAIL': str(failing)}, capture_output=True, text=True, check=True)
            assert 'Delivered 0 notification bundles' in result.stdout
            with sqlite3.connect(database) as db:
                pending = db.execute("select count(*) from notification_bundles where user_id='mail-user' and status=0").fetchone()[0]
                assert pending >= 1
            mailed_before = mailbox.read_text().count('Subject: Rizzy: New notifications')
            workers = [subprocess.Popen([BINARY, 'deliver-notifications'], cwd=ROOT, env=env,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True) for _ in range(2)]
            counts = []
            for worker in workers:
                stdout, stderr = worker.communicate(timeout=10)
                assert worker.returncode == 0, stderr
                counts.append(int(re.search(r'Delivered (\d+) notification bundles', stdout).group(1)))
            assert sum(counts) == pending, counts
            assert mailbox.read_text().count('Subject: Rizzy: New notifications') == mailed_before + pending
            with sqlite3.connect(database) as db:
                assert db.execute("select count(*) from notification_bundles where user_id='mail-user' and status=0").fetchone()[0] == 0
            mail_size = mailbox.stat().st_size
            call(port, f'/{account}/boards/{board_id}/cards', 'POST', {'card': {'title': 'Cancelled account card'}}, bearer=owner_token, status=201)
            time.sleep(2)
            cutoff = (datetime.now(timezone.utc) - timedelta(seconds=1)).isoformat(timespec='milliseconds').replace('+00:00', 'Z')
            with sqlite3.connect(database) as db:
                db.execute("update notification_bundles set ends_at=? where user_id='mail-user' and status=0", (cutoff,))
                db.execute("insert into account_cancellations(id,account_id,initiated_by_id,created_at,updated_at) values('cancelled-mail-test',?,?,datetime('now'),datetime('now'))", (account_id, owner_id))
            result = subprocess.run([BINARY, 'deliver-notifications'], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
            assert 'Delivered 1 notification bundles' in result.stdout
            assert mailbox.stat().st_size == mail_size
            with sqlite3.connect(database) as db:
                assert db.execute("select status from notification_bundles where user_id='mail-user' order by created_at desc limit 1").fetchone()[0] == 2
            print('PASS notification bundle queue, windows, sendmail, unsubscribe, disable, retry, concurrent delivery, cancellation')
        finally:
            stop(server)


if __name__ == '__main__':
    main()
