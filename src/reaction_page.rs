use super::*;

const EMOJI: &[&str] = &[
    "👏", "👍", "🙌", "💪", "🤘", "✊", "✨", "❤️", "💯", "🎉", "🤩", "🥳", "😊", "😀", "😂", "😅",
    "😎", "😉", "😜", "😬", "😮", "😳", "🤔", "😒", "😢", "😭", "😱", "👀", "🙏", "💩", "👎", "✌️",
    "👈", "👆", "✋", "👋", "☀️", "🌙", "💥", "🔥", "🎂", "🍴", "💰",
];

pub(super) async fn card_new(
    State(app): Shared,
    Path((account, number)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let card = card(&app, &p, &number).await?;
    render(
        &headers,
        &account,
        &number,
        "card",
        &card.get::<String, _>("id"),
        None,
    )
}

pub(super) async fn comment_new(
    State(app): Shared,
    Path((account, number, comment_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    let card = card(&app, &p, &number).await?;
    let comment = comment_row(&app, &card, &comment_id).await?;
    render(
        &headers,
        &account,
        &number,
        "comment",
        &comment.get::<String, _>("id"),
        Some(&comment_id),
    )
}

fn render(
    headers: &HeaderMap,
    account: &str,
    number: &str,
    kind: &str,
    target_id: &str,
    comment_id: Option<&str>,
) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let action = if let Some(comment_id) = comment_id {
        format!("/{account}/cards/{number}/comments/{comment_id}/reactions")
    } else {
        format!("/{account}/cards/{number}/reactions")
    };
    let mut buttons = String::new();
    for emoji in EMOJI {
        buttons.push_str(&format!(
            "<button type=\"button\" class=\"reaction__emoji-btn\" data-emoji=\"{emoji}\">{emoji}</button>"
        ));
    }
    let frame = format!(
        "<turbo-frame id=\"{kind}_{}_new_reaction\"><form id=\"reaction_form\" class=\"reaction reaction__form expanded\" aria-label=\"New reaction\" action=\"{action}\" method=\"post\"><label for=\"reaction_content\">Add a reaction</label><input id=\"reaction_content\" class=\"reaction__input\" type=\"text\" name=\"reaction[content]\" maxlength=\"16\" pattern=\"\\S+.*\" autocomplete=\"off\" required autofocus><div class=\"reaction__emoji-list\">{buttons}</div><div class=\"actions\"><button type=\"submit\">Submit</button><a href=\"{action}\">Cancel</a></div></form></turbo-frame>",
        escape(target_id)
    );
    let page = format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>New reaction | Rizzy</title><style>:root{{font:16px/1.5 system-ui,sans-serif;background:#1b1b1b;color:#f5f0e8}}*{{box-sizing:border-box}}body{{margin:0}}main{{max-width:620px;margin:36px auto;padding:24px;border:1px solid #46433d;border-radius:14px;background:#292826}}a{{color:#e7bd61}}input{{display:block;width:100%;margin:12px 0;padding:10px;border:1px solid #666158;border-radius:8px;background:#34322f;color:#fff;font:inherit}}.reaction__emoji-list{{display:grid;grid-template-columns:repeat(8,1fr);gap:5px}}button{{padding:8px;border:0;border-radius:7px;background:#46433d;color:#fff;font:inherit;cursor:pointer}}button:hover{{background:#625c52}}.reaction__emoji-btn{{font-size:22px}}.actions{{display:flex;gap:15px;align-items:center;margin-top:16px}}</style></head><body><main><p><a href="/{account}/cards/{number}">← Back to card</a></p>{frame}<p role="alert" id="reaction_error"></p></main><script>document.querySelectorAll('[data-emoji]').forEach(button=>button.addEventListener('click',()=>{{document.getElementById('reaction_content').value=button.dataset.emoji;document.getElementById('reaction_content').focus()}}));document.getElementById('reaction_form').addEventListener('submit',async event=>{{event.preventDefault();const response=await fetch('{action}',{{method:'POST',headers:{{'Content-Type':'application/json','Accept':'application/json'}},body:JSON.stringify({{reaction:{{content:document.getElementById('reaction_content').value}}}})}});if(response.ok)location.href='/{account}/cards/{number}';else document.getElementById('reaction_error').textContent='Could not add reaction'}})</script></body></html>"##
    );
    Ok(Html(page).into_response())
}
