use axum::response::Html;

use super::escape;

fn page(title: &str, content: &str) -> Html<String> {
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{} | Rizzy</title><style>:root{{font:16px/1.5 system-ui,sans-serif;color:#f5f0e8;background:#1b1b1b}}*{{box-sizing:border-box}}body{{margin:0}}main{{max-width:500px;margin:12vh auto;padding:0 20px}}.panel{{background:#292826;border:1px solid #514d45;border-radius:14px;padding:32px}}h1{{margin-top:0}}form{{display:grid;gap:14px}}input{{width:100%;font:inherit;color:#fff;background:#393631;border:1px solid #777;border-radius:8px;padding:12px}}button{{font:inherit;background:#493a20;color:#fff;border:1px solid #9d844f;border-radius:8px;padding:12px;cursor:pointer}}a{{color:#e7bd61}}.error{{color:#ffae9d}}</style></head><body><main><section class=\"panel\"><h1>{}</h1>{}</section></main></body></html>",
        escape(title),
        escape(title),
        content
    ))
}

pub(super) fn new(rate_limited: bool) -> Html<String> {
    let alert = if rate_limited {
        "<turbo-frame id=\"flash\"><div class=\"flash\"><div class=\"flash__inner shadow\">Try again later.</div></div></turbo-frame>"
    } else {
        "<turbo-frame id=\"flash\"></turbo-frame>"
    };
    page(
        "Sign up",
        &format!(
            "{alert}<form method=\"post\" action=\"/signup\"><input type=\"email\" name=\"signup[email_address]\" autocomplete=\"username\" placeholder=\"Enter your email address…\" required autofocus><p>Enter your email to create an account.</p><button id=\"log_in\" type=\"submit\">Let’s go →</button></form>"
        ),
    )
}

pub(super) fn completion(error: Option<&str>) -> Html<String> {
    let error = error.map(|value|format!("<div class=\"margin-block-half txt-negative txt-small error\" style=\"text-align: left;\"><p class=\"margin-block-none font-weight-bold\">Your changes couldn't be saved:</p><ul class=\"margin-block-none\"><li>{}</li></ul></div>",escape(value))).unwrap_or_default();
    page(
        "Complete your sign-up",
        &format!(
            "<form method=\"post\" action=\"/signup/completion\"><input type=\"text\" name=\"signup[full_name]\" autocomplete=\"name\" placeholder=\"Enter your full name…\" maxlength=\"240\" required autofocus><p>You're one step away. Just enter your name to get your own Rizzy account.</p>{error}<button type=\"submit\">Continue →</button></form><p><a href=\"/account/imports/new\">Or import a Fizzy account</a></p>"
        ),
    )
}
