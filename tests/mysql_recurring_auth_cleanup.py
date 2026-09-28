#!/usr/bin/env python3
"""Check recurring authentication cleanup commands on disposable MySQL 8."""

import os
import subprocess
import tempfile
import uuid
from datetime import datetime, timedelta, timezone
from pathlib import Path

from mysql_webhook_cleanup import MYSQL, ROOT, SOCKET, root_mysql


BINARY = ROOT / "target/debug/rizzy"


def stamp(offset):
    return (datetime.now(timezone.utc) + offset).isoformat(
        timespec="milliseconds").replace("+00:00", "Z")


def main():
    if not SOCKET.is_socket():
        raise SystemExit("Start the disposable MySQL 8 server first")
    subprocess.run(["cargo", "build", "--locked", "--quiet"], cwd=ROOT, check=True)
    suffix = uuid.uuid4().hex[:12]
    database = "rzrec_" + suffix
    user = "rzrec_" + suffix
    password = uuid.uuid4().hex
    root_mysql(f"CREATE DATABASE `{database}` CHARACTER SET utf8mb4 COLLATE utf8mb4_bin; "
               f"CREATE USER '{user}'@'127.0.0.1' IDENTIFIED BY '{password}'; "
               f"GRANT ALL PRIVILEGES ON `{database}`.* TO '{user}'@'127.0.0.1';")
    try:
        with tempfile.TemporaryDirectory(prefix="rizzy-mysql-recurring-") as directory:
            exports = Path(directory) / "exports"
            imports = Path(directory) / "imports"
            exports.mkdir()
            imports.mkdir()
            env = {**os.environ, "RIZZY_DATABASE_URL": f"mysql://{user}:{password}@127.0.0.1:33080/{database}",
                   "RIZZY_STORAGE_DIR": str(Path(directory) / "storage"),
                   "RIZZY_EXPORT_DIR": str(exports), "RIZZY_IMPORT_DIR": str(imports),
                   "RIZZY_SEED_EMAIL": "recurring@example.test",
                   "LD_LIBRARY_PATH": str(MYSQL.resolve().parents[1] / "lib")}
            subprocess.run([BINARY, "seed"], cwd=ROOT, env=env,
                           capture_output=True, text=True, check=True)
            identity = root_mysql(f"SELECT id FROM `{database}`.identities "
                                  "WHERE email_address='recurring@example.test'")
            assert identity and "\n" not in identity, identity
            old = stamp(-timedelta(hours=5))
            future = stamp(timedelta(hours=1))
            recent = stamp(-timedelta(hours=3))
            root_mysql(f"INSERT INTO `{database}`.magic_links(id,identity_id,code,purpose,expires_at,created_at) "
                       f"VALUES('magic-old','{identity}','oldcode','sign_in','{old}','{old}'),"
                       f"('magic-new','{identity}','newcode','sign_in','{future}','{recent}'); "
                       f"INSERT INTO `{database}`.identity_transfers(id,identity_id,token,created_at) "
                       f"VALUES('transfer-old','{identity}','token-old','{old}'),"
                       f"('transfer-new','{identity}','token-new','{recent}');")
            subprocess.run([BINARY, "cleanup-magic-links"], cwd=ROOT, env=env,
                           capture_output=True, text=True, check=True)
            assert root_mysql(f"SELECT id FROM `{database}`.magic_links ORDER BY id") == "magic-new"
            assert root_mysql(f"SELECT COUNT(*) FROM `{database}`.identity_transfers") == "2"
            subprocess.run([BINARY, "cleanup-transfers"], cwd=ROOT, env=env,
                           capture_output=True, text=True, check=True)
            assert root_mysql(f"SELECT id FROM `{database}`.identity_transfers ORDER BY id") == "transfer-new"
            account = root_mysql(f"SELECT id FROM `{database}`.accounts WHERE external_account_id=1")
            owner = root_mysql(f"SELECT id FROM `{database}`.users WHERE account_id='{account}' AND role='owner'")
            older = stamp(-timedelta(hours=25))
            newer = stamp(-timedelta(hours=23))
            root_mysql(f"INSERT INTO `{database}`.exports(id,account_id,user_id,kind,status,created_at,updated_at,completed_at) "
                       f"VALUES('export-old','{account}','{owner}','user','completed','{older}','{older}','{older}'),"
                       f"('export-new','{account}','{owner}','user','completed','{newer}','{newer}','{newer}'); "
                       f"INSERT INTO `{database}`.imports(id,account_id,identity_id,status,created_at,updated_at,completed_at) "
                       f"VALUES('import-old','{account}','{identity}','completed','{older}','{older}','{older}'),"
                       f"('import-new','{account}','{identity}','completed','{newer}','{newer}','{newer}');")
            for name in ("export-old", "export-new"):
                (exports / f"{name}.zip").write_bytes(b"archive")
            for name in ("import-old", "import-new"):
                (imports / f"{name}.zip").write_bytes(b"archive")
            subprocess.run([BINARY, "cleanup-exports"], cwd=ROOT, env=env,
                           capture_output=True, text=True, check=True)
            subprocess.run([BINARY, "cleanup-imports"], cwd=ROOT, env=env,
                           capture_output=True, text=True, check=True)
            assert root_mysql(f"SELECT id FROM `{database}`.exports ORDER BY id") == "export-new"
            assert root_mysql(f"SELECT id FROM `{database}`.imports ORDER BY id") == "import-new"
            assert not (exports / "export-old.zip").exists() and (exports / "export-new.zip").exists()
            assert not (imports / "import-old.zip").exists() and (imports / "import-new.zip").exists()
            failed_env = {**env, "RIZZY_SEED_EMAIL": "failed-import@example.test"}
            subprocess.run([BINARY, "seed"], cwd=ROOT, env=failed_env,
                           capture_output=True, text=True, check=True)
            failed_identity = root_mysql(f"SELECT id FROM `{database}`.identities "
                                         "WHERE email_address='failed-import@example.test'")
            failed_account = root_mysql(f"SELECT account_id FROM `{database}`.users "
                                        f"WHERE identity_id='{failed_identity}' AND role='owner'")
            failed_age = stamp(-timedelta(days=8))
            root_mysql(f"INSERT INTO `{database}`.imports(id,account_id,identity_id,status,"
                       f"failure_reason,created_at,updated_at) VALUES('import-failed',"
                       f"'{failed_account}','{failed_identity}','failed','invalid_export',"
                       f"'{failed_age}','{failed_age}')")
            (imports / "import-failed.zip").write_bytes(b"invalid")
            subprocess.run([BINARY, "cleanup-imports"], cwd=ROOT, env=env,
                           capture_output=True, text=True, check=True)
            assert root_mysql(f"SELECT COUNT(*) FROM `{database}`.accounts WHERE id='{failed_account}'") == "0"
            assert root_mysql(f"SELECT COUNT(*) FROM `{database}`.accounts WHERE id='{account}'") == "1"
            assert not (imports / "import-failed.zip").exists()
    finally:
        root_mysql(f"DROP DATABASE IF EXISTS `{database}`; DROP USER IF EXISTS '{user}'@'127.0.0.1';")
    print("MySQL recurring cleanup retained fresh records and removed expired exports and imports, including a failed import account")


if __name__ == "__main__":
    main()
