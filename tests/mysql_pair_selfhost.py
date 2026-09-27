#!/usr/bin/env python3
"""Run paired parity suites on disposable Fizzy and Rizzy MySQL databases.

Requires the local MySQL server and its socket-authenticated root account.
"""

import argparse
import http.cookiejar
import json
import os
import subprocess
import sys
import tempfile
import uuid
import urllib.request
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "bench"))

from sustained_mixed import (first_difference, prepare_rust, prepare_source,  # noqa: E402
                             start_server, stop_server)
from sustained_mixed_mysql import (create_databases, drop_databases, mysql_query,  # noqa: E402
                                   rust_env, source_env)
from mysql_search_parity import browser_login, run as search_run  # noqa: E402
from parity import run as core_run  # noqa: E402
from board_filter_html_parity import run as board_filter_run  # noqa: E402
from cards_html_parity import check as all_cards_check  # noqa: E402
from named_column_html_parity import run as named_column_run  # noqa: E402
from column_cards_etag_parity import run as column_cache_run  # noqa: E402
from named_column_show_etag_parity import run as column_show_cache_run  # noqa: E402
from draft_workflow_parity import NoRedirect, login, run as draft_run  # noqa: E402
from card_image_parity import IMAGE, run as card_image_run  # noqa: E402
from embed_parity import run as embed_run  # noqa: E402
from card_controls_parity import check as card_controls_check  # noqa: E402
from card_board_html_parity import check as card_board_check  # noqa: E402
from card_edit_html_parity import check as card_edit_check  # noqa: E402
from assignment_html_parity import check as assignment_check  # noqa: E402
from reaction_html_parity import check as reaction_check  # noqa: E402
from column_turbo_parity import check as column_turbo_check  # noqa: E402
from card_previews_html_parity import run as card_previews_run  # noqa: E402
from account_card_create_parity import check as account_card_check  # noqa: E402
from board_views_parity import run as board_views_run  # noqa: E402
from board_settings_html_parity import run as board_settings_run  # noqa: E402
from webhooks_html_parity import check as webhooks_html_check  # noqa: E402
from step_read_parity import run as step_read_run  # noqa: E402
from public_page_html_parity import run as public_page_run  # noqa: E402
from join_code_html_parity import check as join_code_check  # noqa: E402
from users_events_html_parity import check as user_events_check  # noqa: E402
from route_get_status_parity import run as route_get_run  # noqa: E402
from route_get_valid_parity import run as route_get_valid_run, run_html as route_get_valid_html_run  # noqa: E402
from filters_paired_parity import check as filters_browser_check  # noqa: E402
from signup_paired_parity import (check as signup_browser_check,  # noqa: E402
                                  check_completion_cluster as signup_completion_cluster_check,
                                  check_rate_limit as signup_rate_limit_check,
                                  check_rate_limit_cluster as signup_rate_limit_cluster_check)
from passkeys_html_parity import check as passkeys_html_check  # noqa: E402
from auth_format_parity import run as auth_format_run  # noqa: E402
from account_cancellation_mysql_parity import (age_cancellation,  # noqa: E402
                                               concurrent_rust_incineration,
                                               concurrent_rust_mail_delivery,
                                               deliver_rust_cancellation_mail,
                                               normalized_mail, retained_counts,
                                               run as account_cancellation_run, seed_survivor,
                                               source_cancellation_mail, survivor_access,
                                               source_incineration, source_purge_enqueued_blob,
                                               wait_for_deletion)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("suite", choices=("core", "search", "board-filter", "all-cards",
                                          "named-column", "column-cache", "column-show-cache",
                                          "draft", "card-image", "embeds", "card-controls",
                                          "card-board", "card-edit", "assignment", "reaction",
                                          "column-turbo", "card-previews", "account-card",
                                          "board-views", "board-settings", "webhooks-html",
                                          "step-read", "public-page", "join-code",
                                          "user-events", "route-get", "route-get-contract",
                                          "route-get-html", "route-get-valid", "route-get-valid-html",
                                          "filters-browser", "signup-browser", "signup-rate-limit",
                                          "signup-rate-limit-invalid", "signup-rate-limit-json",
                                          "signup-rate-limit-cluster",
                                          "signup-completion-cluster",
                                          "passkeys-html",
                                          "passkeys-flow", "auth-format", "account-cancellation",
                                          "account-cancellation-retention",
                                          "account-cancellation-concurrent",
                                          "account-cancellation-mail",
                                          "account-cancellation-mail-concurrent"))
    args = parser.parse_args()
    client = os.environ.get("RIZZY_TEST_MYSQL_CLIENT", "/tmp/rizzy-mysql8/bin/mysql")
    socket = os.environ.get("RIZZY_TEST_MYSQL_SOCKET", "/tmp/rizzy-mysql8/mysql.sock")
    host = os.environ.get("MYSQL_HOST", "127.0.0.1")
    port = int(os.environ.get("MYSQL_PORT", "33080"))
    suffix = uuid.uuid4().hex[:12]
    source_dbs = {role: f"rzpair_{suffix}_f{label}" for role, label in
                  (("", ""), ("CABLE_", "_c"), ("CACHE_", "_k"), ("QUEUE_", "_q"))}
    rust_db = f"rzpair_{suffix}_r"
    databases = [*source_dbs.values(), rust_db]
    processes = []
    source = rust = None
    created = False
    subprocess.run(["cargo", "build", "--release", "--quiet"], cwd=ROOT, check=True)
    with tempfile.TemporaryDirectory(prefix="rizzy-mysql-pair-selfhost-") as temp:
        root = Path(temp)
        try:
            created = True
            create_databases(client, socket, databases)
            fizzy_dir, rizzy_dir = root / "fizzy", root / "rizzy"
            fizzy_dir.mkdir()
            rizzy_dir.mkdir()
            source_settings = source_env(fizzy_dir, 1, host, port, source_dbs)
            if args.suite in ("signup-browser", "signup-rate-limit", "signup-rate-limit-invalid", "signup-rate-limit-json", "signup-rate-limit-cluster", "signup-completion-cluster", "account-cancellation", "account-cancellation-retention",
                              "account-cancellation-concurrent", "account-cancellation-mail",
                              "account-cancellation-mail-concurrent"):
                source_settings["MULTI_TENANT"] = "true"
            fizzy_env, fizzy_credentials = prepare_source(fizzy_dir, 1, source_settings)
            local_rust_env = rust_env(rizzy_dir, host, port, rust_db)
            if args.suite in ("signup-browser", "signup-rate-limit", "signup-rate-limit-invalid", "signup-rate-limit-json", "signup-rate-limit-cluster", "signup-completion-cluster", "account-cancellation", "account-cancellation-retention",
                              "account-cancellation-concurrent", "account-cancellation-mail",
                              "account-cancellation-mail-concurrent"):
                local_rust_env["RIZZY_MULTI_TENANT"] = "1"
            local_rust_env["RIZZY_SEED_EMAIL"] = (
                "owner@example.com" if args.suite == "route-get-valid"
                else "mysql-pair-owner@example.test")
            local_rust_env["RIZZY_DEV_MAGIC_LINKS"] = "1"
            rizzy_env, rizzy_credentials = prepare_rust(rizzy_dir, local_rust_env)
            cpus = sorted(os.sched_getaffinity(0))[:2]
            fizzy_process, fizzy_url = start_server("fizzy", fizzy_dir, fizzy_env, cpus)
            processes.append(fizzy_process)
            rizzy_process, rizzy_url = start_server("rizzy", rizzy_dir, rizzy_env, cpus)
            processes.append(rizzy_process)
            source = {"name": "fizzy", "url": fizzy_url,
                      "account": str(fizzy_credentials["account"]),
                      "storage_dir": ROOT / "upstream/storage/production/files",
                      "token": fizzy_credentials["token"], "email": "owner@example.com",
                      "mysql_client": client, "mysql_database": source_dbs[""],
                      "queue_database": source_dbs["QUEUE_"],
                      "mysql_host": host, "mysql_port": port, "mysql_user": "rizzy",
                      "mysql_password": "rizzy", "mysql_socket": socket}
            rust = {"name": "rizzy", "url": rizzy_url,
                    "account": str(rizzy_credentials["account"]),
                    "storage_dir": rizzy_dir / "storage",
                    "token": rizzy_credentials["token"],
                    "email": local_rust_env["RIZZY_SEED_EMAIL"],
                    "mysql_client": client, "mysql_database": rust_db,
                    "mysql_host": host, "mysql_port": port, "mysql_user": "rizzy",
                    "mysql_password": "rizzy", "mysql_socket": socket}
            if args.suite == "search":
                browser_login(source)
                browser_login(rust)
                source_result = search_run(source, uuid.uuid4().hex[:8])
                rust_result = search_run(rust, uuid.uuid4().hex[:8])
            elif args.suite == "board-filter":
                browser_login(source)
                browser_login(rust)
                source_result = board_filter_run(source, signed_in=True)
                rust_result = board_filter_run(rust, signed_in=True)
            elif args.suite == "all-cards":
                browser_login(source)
                browser_login(rust)
                source["mysql"] = rust["mysql"] = True
                source_result = all_cards_check(source, signed_in=True)
                rust_result = all_cards_check(rust, signed_in=True)
            elif args.suite == "named-column":
                browser_login(source)
                browser_login(rust)
                source_result = named_column_run(source, signed_in=True)
                rust_result = named_column_run(rust, signed_in=True)
            elif args.suite == "column-cache":
                source_result = column_cache_run(source)
                rust_result = column_cache_run(rust)
            elif args.suite == "column-show-cache":
                browser_login(source)
                browser_login(rust)
                source_result = column_show_cache_run(source, signed_in=True)
                rust_result = column_show_cache_run(rust, signed_in=True)
            elif args.suite == "draft":
                for target in (source, rust):
                    target["opener"] = urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
                source_result = draft_run(source)
                rust_result = draft_run(rust)
            elif args.suite == "card-image":
                source_result = card_image_run(source)
                rust_result = card_image_run(rust)
            elif args.suite == "embeds":
                source_result = embed_run(source)
                rust_result = embed_run(rust)
            elif args.suite == "board-views":
                source_result = board_views_run(source)
                rust_result = board_views_run(rust)
            elif args.suite == "public-page":
                suffix = uuid.uuid4().hex[:8]
                for target in (source, rust):
                    target["opener"] = urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
                source_result = public_page_run(source, suffix)
                rust_result = public_page_run(rust, suffix)
            elif args.suite in ("board-settings", "webhooks-html"):
                for target in (source, rust):
                    target["opener"] = urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
                check = board_settings_run if args.suite == "board-settings" else webhooks_html_check
                source_result = check(source)
                rust_result = check(rust)
            elif args.suite == "step-read":
                for target in (source, rust):
                    target["opener"] = urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
                source_result = step_read_run(source)
                rust_result = step_read_run(rust)
            elif args.suite == "join-code":
                for target in (source, rust):
                    target["opener"] = urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
                source_result = join_code_check(source)
                rust_result = join_code_check(rust)
            elif args.suite == "user-events":
                for target in (source, rust):
                    target["opener"] = urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
                    request = urllib.request.Request(
                        f"{target['url']}/{target['account']}/users.json",
                        headers={"Authorization": f"Bearer {target['token']}",
                                 "Accept": "application/json"})
                    with urllib.request.urlopen(request, timeout=10) as response:
                        users = json.load(response)
                    target["user_id"] = next(user["id"] for user in users if user["role"] == "owner")
                source_result = user_events_check(source)
                rust_result = user_events_check(rust)
            elif args.suite in ("route-get", "route-get-contract", "route-get-html"):
                detailed = args.suite != "route-get"
                accept = "text/html" if args.suite == "route-get-html" else "application/json"
                if accept == "text/html":
                    for target in (source, rust):
                        target["opener"] = urllib.request.build_opener(
                            urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()),
                            NoRedirect())
                        login(target)
                source_result = route_get_run(source, detailed, accept)
                rust_result = route_get_run(rust, detailed, accept)
            elif args.suite == "route-get-valid":
                source_result = route_get_valid_run(source)
                rust_result = route_get_valid_run(rust)
            elif args.suite == "route-get-valid-html":
                for target in (source, rust):
                    target["opener"] = urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
                    login(target)
                source_result = route_get_valid_html_run(source)
                rust_result = route_get_valid_html_run(rust)
            elif args.suite == "filters-browser":
                for target in (source, rust):
                    target["opener"] = urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
                    login(target)
                source_result = filters_browser_check(source)
                rust_result = filters_browser_check(rust)
            elif args.suite == "signup-browser":
                source_result = signup_browser_check(source)
                rust_result = signup_browser_check(rust)
            elif args.suite in ("signup-rate-limit", "signup-rate-limit-invalid",
                                "signup-rate-limit-json"):
                invalid = args.suite == "signup-rate-limit-invalid"
                json_format = args.suite == "signup-rate-limit-json"
                source_result = signup_rate_limit_check(source, invalid=invalid,
                                                        json_format=json_format)
                rust_result = signup_rate_limit_check(rust, invalid=invalid,
                                                      json_format=json_format)
            elif args.suite in ("signup-rate-limit-cluster", "signup-completion-cluster"):
                fizzy_second = root / "fizzy-second"
                rust_second = root / "rizzy-second"
                fizzy_second.mkdir()
                rust_second.mkdir()
                second_fizzy_env = {**fizzy_env, "PIDFILE": str(fizzy_second / "puma.pid")}
                process, fizzy_second_url = start_server(
                    "fizzy", fizzy_second, second_fizzy_env, cpus)
                processes.append(process)
                process, rust_second_url = start_server(
                    "rizzy", rust_second, rizzy_env, cpus)
                processes.append(process)
                checker = (signup_rate_limit_cluster_check
                           if args.suite == "signup-rate-limit-cluster"
                           else signup_completion_cluster_check)
                source_result = checker(source, [fizzy_url, fizzy_second_url])
                rust_result = checker(rust, [rizzy_url, rust_second_url])
            elif args.suite in ("account-cancellation", "account-cancellation-retention",
                               "account-cancellation-concurrent", "account-cancellation-mail",
                               "account-cancellation-mail-concurrent"):
                for target in (source, rust):
                    target["opener"] = urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
                    login(target)
                image_fixture = args.suite in ("account-cancellation-retention",
                                               "account-cancellation-concurrent")
                source_result = account_cancellation_run(source, image=image_fixture)
                rust_result = account_cancellation_run(rust, image=image_fixture)
                if args.suite in ("account-cancellation-mail",
                                  "account-cancellation-mail-concurrent"):
                    source_mail_count = mysql_query(client, socket, source["queue_database"],
                        "SELECT COUNT(*) FROM solid_queue_jobs WHERE class_name='ActionMailer::MailDeliveryJob' "
                        "AND arguments LIKE '%AccountMailer%' AND arguments LIKE '%cancellation%'")
                    assert source_mail_count == "1", ("fizzy", "queued cancellation mail", source_mail_count)
                    source_mail = source_cancellation_mail(fizzy_env)
                    rust_mail = (concurrent_rust_mail_delivery(rust, rizzy_env, rizzy_dir)
                                 if args.suite == "account-cancellation-mail-concurrent" else
                                 deliver_rust_cancellation_mail(rust, rizzy_env, rizzy_dir))
                    source_result["mail_to_owner"] = source_mail["to"] == [source["email"]]
                    rust_result["mail_to_owner"] = rust_mail["to"] == rust["email"]
                    source_result["mail"] = normalized_mail(source_mail, source=True)
                    rust_result["mail"] = normalized_mail(rust_mail)
                    assert source_result["mail_to_owner"] and rust_result["mail_to_owner"]
                if image_fixture:
                    source_survivor_token = seed_survivor(source, fizzy_env, image=True)
                    rust_survivor_token = seed_survivor(rust, rizzy_env, image=True)
                    for target in (source, rust):
                        age_cancellation(target, 29)
                    source_incineration(fizzy_env)
                    stop_server(rizzy_process)
                    rizzy_process, rust["url"] = start_server("rizzy", rizzy_dir, rizzy_env, cpus)
                    processes.append(rizzy_process)
                    source_result["day_29"] = retained_counts(source)
                    rust_result["day_29"] = retained_counts(rust)
                    for target, result in ((source, source_result), (rust, rust_result)):
                        assert result["day_29"] == (1, 1, 1, 1, 1, 1), (target["name"], result)
                        result["day_29_files"] = (
                            target["cancelled_image_path"].read_bytes() == IMAGE,
                            target["survivor_image_path"].read_bytes() == IMAGE)
                        assert result["day_29_files"] == (True, True), (target["name"], result)
                    for target in (source, rust):
                        age_cancellation(target, 31)
                    source_incineration(fizzy_env)
                    if args.suite == "account-cancellation-concurrent":
                        concurrent_rust_incineration(rust, rizzy_env)
                    else:
                        subprocess.run([str(ROOT / "target/release/rizzy"), "incinerate-due"],
                                       cwd=ROOT, env=rizzy_env, capture_output=True,
                                       text=True, check=True)
                    source_result["day_31"] = wait_for_deletion(source)
                    rust_result["day_31"] = wait_for_deletion(rust)
                    for target, result in ((source, source_result), (rust, rust_result)):
                        assert result["day_31"] == (0, 1, 0, 0, 1, 1), (target["name"], result)
                    assert source["cancelled_image_path"].exists(), "Fizzy purge ran before queued job"
                    source_purge_enqueued_blob(fizzy_env, source["cancelled_image_key"])
                    for target, result in ((source, source_result), (rust, rust_result)):
                        result["day_31_files"] = (
                            target["cancelled_image_path"].exists(),
                            target["survivor_image_path"].read_bytes() == IMAGE)
                        assert result["day_31_files"] == (False, True), (target["name"], result)
                    source_result["survivor_access"] = survivor_access(source, source_survivor_token)
                    rust_result["survivor_access"] = survivor_access(rust, rust_survivor_token)
            elif args.suite == "passkeys-html":
                for target in (source, rust):
                    target["opener"] = urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
                source_result = passkeys_html_check(source)
                rust_result = passkeys_html_check(rust)
            elif args.suite == "auth-format":
                for target in (source, rust):
                    request = urllib.request.Request(
                        f"{target['url']}/{target['account']}/boards.json",
                        data=json.dumps({"board": {"name": "Auth format fixture"}}).encode(),
                        method="POST",
                        headers={"Authorization": f"Bearer {target['token']}",
                                 "Accept": "application/json",
                                 "Content-Type": "application/json"})
                    with urllib.request.urlopen(request, timeout=10) as response:
                        assert response.status == 201, target["name"]
                source_result = auth_format_run(source)
                rust_result = auth_format_run(rust)
            elif args.suite == "passkeys-flow":
                source_test_env = {**os.environ, "PASSKEY_TARGET": "fizzy",
                                   "FIZZY_URL": source["url"], "FIZZY_EMAIL": source["email"],
                                   "FIZZY_ACCOUNT": source["account"],
                                   "FIZZY_MYSQL_DATABASE": source_dbs[""],
                                   "FIZZY_MYSQL_CLIENT": client,
                                   "FIZZY_MYSQL_HOST": host,
                                   "FIZZY_MYSQL_PORT": str(port)}
                rust_test_env = {**os.environ, "PASSKEY_TARGET": "rizzy",
                                 "RIZZY_URL": rust["url"].replace("127.0.0.1", "localhost"),
                                 "RIZZY_EMAIL": rust["email"],
                                 "RIZZY_ACCOUNT": rust["account"],
                                 "RIZZY_TEST_MYSQL_URL": local_rust_env["RIZZY_DATABASE_URL"],
                                 "RIZZY_TEST_MYSQL_CLIENT": client}
                source_flow = subprocess.run(
                    ["python3", "tests/passkey_parity.py"], cwd=ROOT,
                    env=source_test_env, capture_output=True, text=True)
                assert source_flow.returncode == 0, source_flow.stdout + source_flow.stderr
                source_result = source_flow.stdout.strip()
                rust_flow = subprocess.run(
                    ["python3", "tests/passkey_parity.py"], cwd=ROOT,
                    env=rust_test_env, capture_output=True, text=True)
                assert rust_flow.returncode == 0, rust_flow.stdout + rust_flow.stderr
                rust_result = rust_flow.stdout.strip()
            elif args.suite in ("card-controls", "card-board", "card-edit",
                                 "assignment", "reaction", "column-turbo",
                                 "card-previews", "account-card"):
                for target in (source, rust):
                    target["opener"] = urllib.request.build_opener(
                        urllib.request.HTTPCookieProcessor(http.cookiejar.CookieJar()), NoRedirect())
                if args.suite == "column-turbo":
                    source_result = column_turbo_check("fizzy", target_override=source)
                    rust_result = column_turbo_check("rizzy", target_override=rust)
                    for result in (source_result, rust_result):
                        for key in ("create", "move_left", "move_right", "update"):
                            result[key] = [action for action, _ in result[key]]
                else:
                    check = {"card-controls": card_controls_check,
                             "card-board": card_board_check,
                             "card-edit": card_edit_check,
                             "assignment": assignment_check,
                             "reaction": reaction_check,
                             "card-previews": card_previews_run,
                             "account-card": account_card_check}[args.suite]
                    source_result = check(source)
                    rust_result = check(rust)
            else:
                source_result = core_run(source)
                rust_result = core_run(rust)
            difference = first_difference(source_result, rust_result)
            if args.suite in ("route-get", "route-get-contract", "route-get-html", "route-get-valid", "route-get-valid-html") and difference is not None:
                for path in source_result.keys() | rust_result.keys():
                    if source_result.get(path) != rust_result.get(path):
                        print(f"{path}: Fizzy={source_result.get(path)} "
                              f"Rizzy={rust_result.get(path)}", file=sys.stderr)
            assert difference is None, difference
            if args.suite == "search":
                print(f"Disposable MySQL search fixture matched pinned Fizzy: "
                      f"{len(source_result['query_probe'])} query cases, "
                      f"{len(source_result['browser'])} rendered search cases")
            elif args.suite == "board-filter":
                print(f"Disposable MySQL board filters matched pinned Fizzy: "
                      f"{len(source_result)} browser workflow comparisons")
            elif args.suite == "all-cards":
                print("Disposable MySQL all-cards HTML workflow matched pinned Fizzy")
            elif args.suite == "named-column":
                print("Disposable MySQL named-column HTML workflow matched pinned Fizzy")
            elif args.suite == "column-cache":
                print("Disposable MySQL named-column JSON validators matched pinned Fizzy")
            elif args.suite == "column-show-cache":
                print("Disposable MySQL named-column show validators matched pinned Fizzy")
            elif args.suite == "draft":
                print("Disposable MySQL draft browser workflow matched pinned Fizzy")
            elif args.suite == "card-image":
                print("Disposable MySQL card image workflow matched pinned Fizzy; "
                      f"image removal timestamp changes={source_result['delete_touch']}")
            elif args.suite == "embeds":
                print("Disposable MySQL rich-text embed workflow matched pinned Fizzy")
            elif args.suite == "board-views":
                print(f"Disposable MySQL private and public board views matched pinned Fizzy: "
                      f"{len(source_result)} comparisons")
            elif args.suite in ("board-settings", "webhooks-html"):
                print(f"Disposable MySQL {args.suite} browser workflow matched pinned Fizzy")
            elif args.suite in ("step-read", "public-page"):
                print(f"Disposable MySQL {args.suite} workflow matched pinned Fizzy")
            elif args.suite == "join-code":
                print("Disposable MySQL invite-link browser and QR workflow matched pinned Fizzy")
            elif args.suite == "user-events":
                print("Disposable MySQL user activity browser workflow matched pinned Fizzy")
            elif args.suite in ("route-get", "route-get-contract", "route-get-html"):
                print(f"Disposable MySQL GET error contracts matched pinned Fizzy: "
                      f"{len(source_result)} routes")
            elif args.suite == "route-get-valid":
                print(f"Disposable MySQL populated JSON route values and write validators matched pinned Fizzy: "
                      f"{len(source_result)} probes")
            elif args.suite == "route-get-valid-html":
                print(f"Disposable MySQL populated HTML GET contracts matched pinned Fizzy: "
                      f"{len(source_result)} probes")
            elif args.suite == "filters-browser":
                print("Disposable MySQL saved-filter browser lifecycle matched pinned Fizzy")
            elif args.suite == "signup-browser":
                print("Disposable MySQL multi-tenant browser signup matched pinned Fizzy")
            elif args.suite in ("signup-rate-limit", "signup-rate-limit-invalid",
                                "signup-rate-limit-json"):
                print(f"Disposable MySQL {args.suite} matched pinned Fizzy")
            elif args.suite == "signup-rate-limit-cluster":
                print("Two-process MySQL signup burst matched pinned Fizzy and honored the shared limit")
            elif args.suite == "signup-completion-cluster":
                print("Two-process MySQL account creation matched pinned Fizzy without duplicate account numbers")
            elif args.suite == "account-cancellation":
                print("Disposable MySQL owner cancellation and retained records matched pinned Fizzy")
            elif args.suite == "account-cancellation-retention":
                print("Disposable MySQL cancellation retention, scoped deletion, and queued file cleanup matched pinned Fizzy")
            elif args.suite == "account-cancellation-concurrent":
                print("Two concurrent Rizzy MySQL incineration workers preserved paired Fizzy deletion and file outcomes")
            elif args.suite == "account-cancellation-mail":
                print("Queued Rizzy cancellation mail, retry, and multipart content matched pinned Fizzy")
            elif args.suite == "account-cancellation-mail-concurrent":
                print("Two overlapping Rizzy mail workers delivered one queued cancellation message with Fizzy-matched content")
            elif args.suite == "passkeys-html":
                print("Disposable MySQL account-scoped passkey pages matched pinned Fizzy")
            elif args.suite == "auth-format":
                print("Disposable MySQL bearer format and permission checks matched pinned Fizzy")
            elif args.suite == "passkeys-flow":
                print("Disposable MySQL passkey registration, management, and login matched pinned Fizzy")
            elif args.suite in ("card-controls", "card-board", "card-edit",
                                     "assignment", "reaction", "column-turbo",
                                     "card-previews", "account-card"):
                print(f"Disposable MySQL {args.suite} browser workflow matched pinned Fizzy")
            else:
                print(f"Disposable MySQL core workflow matched pinned Fizzy: "
                      f"{len(source_result)} comparisons")
        except Exception:
            for name in ("fizzy", "rizzy"):
                log = root / name / f"{name}.log"
                if log.exists():
                    print(f"{name} log tail:\n{log.read_text()[-4000:]}", file=sys.stderr)
            raise
        finally:
            for process in processes:
                stop_server(process)
            if source is not None:
                for path in source.get("uploaded_files", []):
                    path.unlink(missing_ok=True)
            if created:
                drop_databases(client, socket, databases)


if __name__ == "__main__":
    main()
