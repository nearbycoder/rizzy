#!/usr/bin/env python3
"""Check the tag cleanup's saved-filter effects on an isolated SQLite account."""

import hashlib
import json
import os
import sqlite3
import subprocess
import sys
import tempfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))
import sustained_mixed  # noqa: E402
from card_workflow import expect  # noqa: E402


def source_filter_effects(directory):
    env, _ = sustained_mixed.prepare_source(directory, 2)
    script = '''require "json"
      account = Account.first
      user = account.users.find_by!(role: "owner")
      Current.account = account
      Current.user = user
      tags = %w[only collision survivor].to_h do |name|
        [ name, Tag.create!(account: account, title: name) ]
      end
      user.filters.remember(tag_ids: [ tags["only"].id ])
      user.filters.remember(indexed_by: "closed", tag_ids: [ tags["collision"].id ])
      user.filters.remember(indexed_by: "closed")
      user.filters.remember(sorted_by: "oldest", tag_ids: [ tags["survivor"].id ])
      DeleteUnusedTagsJob.perform_now
      puts "TAG_CLEANUP_RESULT=" + JSON.generate({
        tags: Tag.count, filters: user.filters.reload.map(&:as_params).sort_by(&:to_s)
      })'''
    result = subprocess.run(["bin/rails", "runner", script], cwd=sustained_mixed.SOURCE,
                            env=env, capture_output=True, text=True, check=True)
    line = next(line for line in result.stdout.splitlines()
                if line.startswith("TAG_CLEANUP_RESULT="))
    return json.loads(line.split("=", 1)[1])


def main():
    sustained_mixed.RUST = ROOT / "target/debug/rizzy"
    with tempfile.TemporaryDirectory(prefix="rizzy-unused-tags-") as temporary:
        directory = Path(temporary)
        env, credentials = sustained_mixed.prepare_rust(directory)
        env["RIZZY_DISABLE_TAG_CLEANUP"] = "1"
        process, url = sustained_mixed.start_server(
            "rizzy", directory, env, sorted(os.sched_getaffinity(0))[:2]
        )
        try:
            target = {"url": url, "account": str(credentials["account"]),
                      "token": credentials["token"]}
            board, _ = expect(target, "/boards", "POST", {"board": {"name": "Tag cleanup"}}, 201)
            card, _ = expect(target, f"/boards/{board['id']}/cards", "POST",
                             {"card": {"title": "Tagged"}}, 201)
            expect(target, f"/cards/{card['number']}/taggings", "POST",
                   {"tag_title": "used"}, 204)
        finally:
            sustained_mixed.stop_server(process)

        database = directory / "rizzy.sqlite3"
        with sqlite3.connect(database) as db:
            account_id = db.execute("SELECT id FROM accounts LIMIT 1").fetchone()[0]
            creator_id = db.execute("SELECT id FROM users WHERE role='owner' LIMIT 1").fetchone()[0]
            used_id = db.execute("SELECT id FROM tags WHERE title='used'").fetchone()[0]
            tags = {"orphan": "cleanup-orphan", "only": "cleanup-only",
                    "collision": "cleanup-collision", "survivor": "cleanup-survivor"}
            for tag_id, title in tags.items():
                db.execute("INSERT INTO tags(id,account_id,title,created_at) VALUES(?,?,?,?)",
                           (tag_id, account_id, title, "2026-01-01T00:00:00Z"))

            def save_filter(filter_id, fields, tag_id=None):
                encoded = json.dumps(fields, sort_keys=True, separators=(",", ":"))
                db.execute("INSERT INTO filters VALUES(?,?,?,?,?,?,?)",
                           (filter_id, account_id, creator_id, encoded,
                            hashlib.sha256(encoded.encode()).hexdigest(),
                            "2026-01-01T00:00:00Z", "2026-01-01T00:00:00Z"))
                if tag_id:
                    db.execute("INSERT INTO filters_tags VALUES(?,?)", (filter_id, tag_id))

            save_filter("tag_only", {"tag_ids": ["only"]}, "only")
            save_filter("colliding", {"indexed_by": "closed", "tag_ids": ["collision"]}, "collision")
            save_filter("closed", {"indexed_by": "closed"})
            save_filter("surviving", {"sorted_by": "oldest", "tag_ids": ["survivor"]}, "survivor")
            db.commit()

        first = subprocess.run([str(sustained_mixed.RUST), "delete-unused-tags"], cwd=ROOT,
                               env=env, capture_output=True, text=True, check=True)
        assert "Deleted 4 unused tags" in first.stdout, first.stdout
        with sqlite3.connect(database) as db:
            assert db.execute("SELECT id FROM tags").fetchall() == [(used_id,)]
            assert db.execute("SELECT id FROM filters ORDER BY id").fetchall() == [
                ("closed",), ("surviving",)]
            fields, digest, updated_at = db.execute(
                "SELECT fields,params_digest,updated_at FROM filters WHERE id='surviving'"
            ).fetchone()
            assert json.loads(fields) == {"sorted_by": "oldest"}, fields
            assert digest == hashlib.sha256(fields.encode()).hexdigest()
            assert updated_at != "2026-01-01T00:00:00Z"
            assert db.execute("SELECT COUNT(*) FROM filters_tags").fetchone()[0] == 0
        repeat = subprocess.run([str(sustained_mixed.RUST), "delete-unused-tags"], cwd=ROOT,
                                env=env, capture_output=True, text=True, check=True)
        assert "Deleted 0 unused tags" in repeat.stdout, repeat.stdout
        source = directory / "source"
        source.mkdir()
        expected = source_filter_effects(source)
        assert expected == {"tags": 0, "filters": [
            {"indexed_by": "closed"}, {"sorted_by": "oldest"}]}, expected
    print("Unused-tag cleanup preserves used tags and matches Fizzy saved-filter effects")


if __name__ == "__main__":
    main()
