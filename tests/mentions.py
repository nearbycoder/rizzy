#!/usr/bin/env python3
"""Exercise live User attachment mentions on cards and comments."""
import hashlib
import os
import re
import sqlite3
import subprocess
import tempfile
from pathlib import Path

from auth import BINARY, ROOT, start, stop
from join import call


def markup(user_id):
    return (f'<action-text-attachment sgid="gid://fizzy/User/{user_id}" '
            'content-type="application/vnd.actiontext.mention"></action-text-attachment>')


def main():
    subprocess.run(["cargo", "build", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory() as temp:
        database = Path(temp) / "mentions.sqlite3"
        env = {**os.environ, "RIZZY_DATABASE_URL": f"sqlite://{database}"}
        seeded = subprocess.run([BINARY, "seed"], cwd=ROOT, env=env, capture_output=True, text=True, check=True)
        account = re.search(r"Account: (\d+)", seeded.stdout).group(1)
        owner_token = re.search(r"Bearer token: (\w+)", seeded.stdout).group(1)
        port, server = start(env)
        try:
            board, _ = call(port, f"/{account}/boards", "POST", {"board": {"name": "Mentions", "all_access": False}}, bearer=owner_token, status=201)
            board_id, owner_id = board["id"], board["creator"]["id"]
            with sqlite3.connect(database) as db:
                account_id = db.execute("SELECT account_id FROM boards WHERE id=?", (board_id,)).fetchone()[0]
                for user_id, name in [("mentioned", "Mentioned User"), ("outsider", "Outside User")]:
                    identity = f"identity-{user_id}"
                    db.execute("INSERT INTO identities(id,email_address,created_at) VALUES(?,?, '2026-01-01T00:00:00Z')",
                               (identity, f"{user_id}@example.com"))
                    db.execute("INSERT INTO users(id,account_id,identity_id,name,role,created_at) VALUES(?,?,?,?, 'member','2026-01-01T00:00:00Z')",
                               (user_id, account_id, identity, name))
                    db.execute("INSERT INTO access_tokens(id,identity_id,token_hash,description,permission,created_at) VALUES(?,?,?,'Test','write','2026-01-01T00:00:00Z')",
                               (f"token-{user_id}", identity, hashlib.sha256(user_id.encode()).hexdigest()))
            call(port, f"/{account}/boards/{board_id}", "PUT", {"user_ids": [owner_id, "mentioned"]}, bearer=owner_token)
            call(port, f"/{account}/boards/{board_id}/involvement", "PUT", {"involvement": "watching"}, bearer="mentioned", status=204)
            description = f'<p>Hello {markup("mentioned")}{markup("mentioned")}{markup("outsider")}{markup(owner_id)}</p>'
            card, _ = call(port, f"/{account}/boards/{board_id}/cards", "POST",
                           {"card": {"title": "Mention card", "description": description}}, bearer=owner_token, status=201)
            card_id, number = card["id"], card["number"]
            with sqlite3.connect(database) as db:
                rows = db.execute("SELECT mentionee_id,source_type FROM mentions WHERE source_id=?", (card_id,)).fetchall()
                assert sorted(rows) == sorted([("mentioned", "Card"), (owner_id, "Card")]), rows
                assert db.execute("SELECT watching FROM watches WHERE card_id=? AND user_id='mentioned'", (card_id,)).fetchone() == (1,)
                assert db.execute("SELECT COUNT(*) FROM notifications WHERE card_id=? AND user_id=? AND source_type='Mention'", (card_id, "mentioned")).fetchone()[0] == 1
                assert db.execute("SELECT COUNT(*) FROM notifications WHERE card_id=? AND user_id=?", (card_id, owner_id)).fetchone()[0] == 0
            notices, _ = call(port, f"/{account}/notifications", bearer="mentioned")
            assert len(notices) == 1 and notices[0]["source_type"] == "mention"
            assert notices[0]["unread_count"] == 1, notices
            assert "Hello MentionedMentioned" in notices[0]["body"], notices
            call(port, f"/{account}/cards/{number}", "PUT", {"card": {"description": description}}, bearer=owner_token)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM mentions WHERE source_id=?", (card_id,)).fetchone()[0] == 2
            comment, _ = call(port, f"/{account}/cards/{number}/comments", "POST",
                              {"comment": {"body": f'<p>Thoughts {markup("mentioned")}</p>'}}, bearer=owner_token, status=201)
            comment_id = comment["id"]
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM mentions WHERE source_type='Comment' AND source_id=? AND mentionee_id='mentioned'", (comment_id,)).fetchone()[0] == 1
            notices, _ = call(port, f"/{account}/notifications", bearer="mentioned")
            assert len(notices) == 1 and notices[0]["source_type"] == "mention" and notices[0]["unread_count"] == 2, notices
            call(port, f"/{account}/boards/{board_id}", "PUT", {"user_ids": [owner_id, "mentioned", "outsider"]}, bearer=owner_token)
            call(port, f"/{account}/cards/{number}/comments/{comment_id}", "PUT",
                 {"comment": {"body": f'<p>Thoughts {markup("mentioned")} and {markup("outsider")}</p>'}}, bearer=owner_token)
            with sqlite3.connect(database) as db:
                rows = db.execute("SELECT mentionee_id FROM mentions WHERE source_type='Comment' AND source_id=? ORDER BY mentionee_id", (comment_id,)).fetchall()
                assert rows == [("mentioned",), ("outsider",)], rows
                assert db.execute("SELECT watching FROM watches WHERE card_id=? AND user_id='outsider'", (card_id,)).fetchone() == (1,)
            outsider_notices, _ = call(port, f"/{account}/notifications", bearer="outsider")
            assert len(outsider_notices) == 1 and outsider_notices[0]["source_type"] == "mention", outsider_notices
            with sqlite3.connect(database) as db:
                draft_number = db.execute("UPDATE accounts SET cards_count=cards_count+1 WHERE id=? RETURNING cards_count", (account_id,)).fetchone()[0]
                db.execute("INSERT INTO cards(id,account_id,board_id,creator_id,number,title,description,status,created_at,updated_at,last_active_at) VALUES('mention-draft',?,?,?,?,?,?,'drafted','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z','2026-01-01T00:00:00Z')",
                           (account_id, board_id, owner_id, draft_number, "Draft mention", f'<p>{markup("outsider")}</p>'))
            call(port, f"/{account}/cards/{draft_number}", "PUT",
                 {"card": {"description": f'<p>Edited {markup("outsider")}</p>'}}, bearer=owner_token)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM mentions WHERE source_id='mention-draft'").fetchone()[0] == 0
            call(port, f"/{account}/cards/{draft_number}/publish", "POST", bearer=owner_token, status=201)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM mentions WHERE source_id='mention-draft' AND mentionee_id='outsider'").fetchone()[0] == 1
            call(port, f"/{account}/boards/{board_id}", "PUT", {"user_ids": [owner_id, "mentioned"]}, bearer=owner_token)
            with sqlite3.connect(database) as db:
                assert db.execute("SELECT COUNT(*) FROM mentions WHERE mentionee_id='outsider'").fetchone()[0] == 0
                assert db.execute("SELECT COUNT(*) FROM notifications WHERE user_id='outsider'").fetchone()[0] == 0
            print("Live mentions passed: access, deduplication, self-mention, watch, notification, comment edit, draft publish, revocation")
        finally:
            stop(server)


if __name__ == "__main__":
    main()
