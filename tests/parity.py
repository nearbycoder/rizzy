#!/usr/bin/env python3
"""Exercise the currently implemented JSON workflow on Fizzy and Rizzy."""
import json
import os
import sys
import time
import urllib.error
import urllib.request


def request(target, path, method="GET", payload=None):
    global_path = path.startswith('/my/identity') or path.startswith('/my/access_tokens')
    root = target['url'] if global_path else f"{target['url']}/{target['account']}"
    req = urllib.request.Request(
        f"{root}{path}",
        data=json.dumps(payload).encode() if payload is not None else None,
        method=method,
        headers={"Authorization": f"Bearer {target['token']}", "Accept": "application/json", "Content-Type": "application/json"},
    )
    try:
        response = urllib.request.urlopen(req, timeout=10)
    except urllib.error.HTTPError as error:
        response = error
    with response:
        raw = response.read()
        return response.status, json.loads(raw) if raw else None


def shape(value):
    if isinstance(value, dict):
        return {key: shape(item) for key, item in value.items()}
    if isinstance(value, list):
        return [shape(value[0])] if value else []
    return type(value).__name__


def run(target):
    result = {}

    def step(label, path, method="GET", payload=None):
        status, value = request(target, path, method, payload)
        result[label] = {"status": status, "shape": shape(value)}
        return value

    identity = step("identity_show", "/my/identity")
    step("account_show","/account/settings")
    result["account_entropy_days"]=step("account_entropy", "/account/entropy", "PUT", {"entropy":{"auto_postpone_period_in_days":7}})["auto_postpone_period_in_days"]
    users = step("users_list", "/users")
    result["users_list"]["count"] = len(users)
    step("user_show", f"/users/{users[0]['id']}")
    token = step("token_create", "/my/access_tokens", "POST", {"access_token": {"description": "Parity", "permission": "read"}})
    step("token_list", "/my/access_tokens")
    step("token_delete", f"/my/access_tokens/{token['id']}", "DELETE")
    board = step("board_create", "/boards", "POST", {"board": {"name": "Parity " + str(time.time_ns())}})
    board_id = board["id"]
    result["inherited_board_days"]=board["auto_postpone_period_in_days"]
    step("board_show", f"/boards/{board_id}")
    column = step("column_create", f"/boards/{board_id}/columns", "POST", {"column": {"name": "Doing"}})
    column_id = column["id"]
    step("column_show", f"/boards/{board_id}/columns/{column_id}")
    search_word = "parityk" + str(time.time_ns())
    card = step("card_create", f"/boards/{board_id}/cards", "POST", {"card": {"title": "Parity " + search_word, "description": "Same text"}})
    number = card["number"]
    result["card_create"]["description_html"] = card["description_html"]
    step("card_show", f"/cards/{number}")
    step("board_entropy",f"/boards/{board_id}/entropy","PUT",{"board":{"auto_postpone_period_in_days":90}})
    step("board_touch_after_entropy",f"/boards/{board_id}","PATCH",{"board":{"name":"Active " + search_word}})
    result["board_entropy_effective"]=step("board_show_after_entropy",f"/boards/{board_id}")["auto_postpone_period_in_days"]
    result["account_entropy_restored_days"]=step("account_entropy_restore","/account/entropy","PUT",{"entropy":{"auto_postpone_period_in_days":30}})["auto_postpone_period_in_days"]
    result["board_override_survives"]=step("board_show_after_account_entropy",f"/boards/{board_id}")["auto_postpone_period_in_days"]
    result["search_text_count"]=len(step("search_text",f"/search?q={search_word}"))
    result["search_number_count"]=len(step("search_number",f"/search?q={number}"))
    listing = step("card_list", f"/cards?board_ids%5B%5D={board_id}")
    result["card_list"]["count"] = len(listing)
    step("triage", f"/cards/{number}/triage", "POST", {"column_id": column_id})
    result["triaged"] = step("triage_show", f"/cards/{number}")["column"]["name"]
    step("assign",f"/cards/{number}/assignments","POST",{"assignee_id":users[0]["id"]})
    result["assigned_count"]=len(step("assign_show",f"/cards/{number}")["assignees"])
    result["assignee_filter_count"]=len(step("assignee_filter",f"/cards?board_ids%5B%5D={board_id}&assignee_ids%5B%5D={users[0]['id']}"))
    step("unassign",f"/cards/{number}/assignments","POST",{"assignee_id":users[0]["id"]})
    result["unassigned_count"]=len(step("unassign_show",f"/cards/{number}")["assignees"])
    result["unassigned_filter_count"]=len(step("unassigned_filter",f"/cards?board_ids%5B%5D={board_id}&assignment_status=unassigned"))
    step("watch",f"/cards/{number}/watch","POST")
    step("unwatch",f"/cards/{number}/watch","DELETE")
    step("pin",f"/cards/{number}/pin","POST")
    result["pin_count"]=len(step("pin_list","/my/pins"))
    step("unpin",f"/cards/{number}/pin","DELETE")
    reaction=step("reaction_create",f"/cards/{number}/reactions","POST",{"reaction":{"content":"👍"}})
    result["reaction_count"]=len(step("reaction_list",f"/cards/{number}/reactions"))
    step("reaction_delete",f"/cards/{number}/reactions/{reaction['id']}","DELETE")
    step("gold", f"/cards/{number}/goldness", "POST")
    result["golden"] = step("gold_show", f"/cards/{number}")["golden"]
    step("close", f"/cards/{number}/closure", "POST")
    result["closed"] = step("close_show", f"/cards/{number}")["closed"]
    result["closed_filter_count"]=len(step("closed_filter",f"/cards?board_ids%5B%5D={board_id}&indexed_by=closed"))
    result["open_after_close_count"]=len(step("open_after_close",f"/cards?board_ids%5B%5D={board_id}"))
    step("reopen", f"/cards/{number}/closure", "DELETE")
    step("postpone", f"/cards/{number}/not_now", "POST")
    result["postponed"] = step("postpone_show", f"/cards/{number}")["postponed"]
    result["not_now_filter_count"]=len(step("not_now_filter",f"/cards?board_ids%5B%5D={board_id}&indexed_by=not_now"))
    result["open_after_postpone_count"]=len(step("open_after_postpone",f"/cards?board_ids%5B%5D={board_id}"))
    step("tag", f"/cards/{number}/taggings", "POST", {"tag_title": "#parity"})
    result["tags"] = step("tag_show", f"/cards/{number}")["tags"]
    tags=step("tags_list","/tags")
    tag_id=next(tag["id"] for tag in tags if tag["title"]=="parity")
    result["tag_filter_count"]=len(step("tag_filter",f"/cards?tag_ids%5B%5D={tag_id}&card_ids%5B%5D={card['id']}"))
    item = step("step_create", f"/cards/{number}/steps", "POST", {"step": {"content": "First step"}})
    step("step_update", f"/cards/{number}/steps/{item['id']}", "PUT", {"step": {"completed": True}})
    result["step_completed"] = step("step_list", f"/cards/{number}/steps")[0]["completed"]
    comment = step("comment_create", f"/cards/{number}/comments", "POST", {"comment": {"body": "First comment"}})
    result["comment_create"]["html"] = comment["body"]["html"]
    comment_reaction=step("comment_reaction_create",f"/cards/{number}/comments/{comment['id']}/reactions","POST",{"reaction":{"content":"👍"}})
    result["comment_reaction_count"]=len(step("comment_reaction_list",f"/cards/{number}/comments/{comment['id']}/reactions"))
    step("comment_reaction_delete",f"/cards/{number}/comments/{comment['id']}/reactions/{comment_reaction['id']}","DELETE")
    step("comment_update", f"/cards/{number}/comments/{comment['id']}", "PUT", {"comment": {"body": "Updated comment"}})
    comments = step("comment_list", f"/cards/{number}/comments")
    result["comment_body"] = next(item["body"]["plain_text"] for item in comments if item["id"] == comment["id"])
    result["system_comment_count"] = sum(item["creator"]["role"] == "system" for item in comments)
    activities=step("activity_list",f"/activities?board_ids%5B%5D={board_id}")
    result["activity_actions"]=sorted(item["action"] for item in activities)
    result["activity_particulars"]={item["action"]:shape(item["particulars"]) for item in activities}
    result["activity_descriptions"]={item["action"]:item["description"].replace(search_word,"<WORD>") for item in activities}
    result["activity_creator_filter_count"]=len(step("activity_creator_filter",f"/activities?board_ids%5B%5D={board_id}&creator_ids%5B%5D={users[0]['id']}"))
    step("card_rename",f"/cards/{number}","PUT",{"card":{"title":"Renamed " + search_word}})
    other_board=step("second_board_create","/boards","POST",{"board":{"name":"Moved " + search_word}})
    step("card_move",f"/cards/{number}/board","PUT",{"board_id":other_board["id"]})
    result["old_board_activity_count"]=len(step("old_board_activities",f"/activities?board_ids%5B%5D={board_id}"))
    moved=step("moved_activities",f"/activities?board_ids%5B%5D={other_board['id']}")
    result["moved_activity_actions"]=sorted(item["action"] for item in moved)
    result["moved_activity_particulars"]={item["action"]:shape(item["particulars"]) for item in moved}
    step("comment_delete", f"/cards/{number}/comments/{comment['id']}", "DELETE")
    result["activities_after_comment_delete"]=sorted(item["action"] for item in step("activity_after_comment_delete",f"/activities?board_ids%5B%5D={other_board['id']}"))
    published=step("board_publish",f"/boards/{board_id}/publication","POST")
    result["published_url_present"]="public_url" in published
    step("board_published_show",f"/boards/{board_id}")
    step("board_unpublish",f"/boards/{board_id}/publication","DELETE")
    result["unpublished_url_absent"]="public_url" not in step("board_unpublished_show",f"/boards/{board_id}")
    step("card_delete", f"/cards/{number}", "DELETE")
    result["activities_after_card_delete_count"]=len(step("activity_after_card_delete",f"/activities?board_ids%5B%5D={other_board['id']}"))
    step("board_delete", f"/boards/{board_id}", "DELETE")
    step("second_board_delete",f"/boards/{other_board['id']}","DELETE")
    return result


def main():
    targets = {}
    for name in ("FIZZY", "RIZZY"):
        fields = ("URL", "ACCOUNT", "TOKEN")
        if any(not os.getenv(f"{name}_{field}") for field in fields):
            sys.exit("Set FIZZY_URL/ACCOUNT/TOKEN and RIZZY_URL/ACCOUNT/TOKEN")
        targets[name.lower()] = {field.lower(): os.environ[f"{name}_{field}"] for field in fields}
    left, right = run(targets["fizzy"]), run(targets["rizzy"])
    differences = {key: {"fizzy": left.get(key), "rizzy": right.get(key)} for key in left.keys() | right.keys() if left.get(key) != right.get(key)}
    print(json.dumps({"checked": len(left), "differences": differences}, indent=2))
    sys.exit(bool(differences))


if __name__ == "__main__":
    main()
