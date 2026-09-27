use axum::response::Html;

use super::escape;

const ARROW: &str = r#"<svg aria-hidden="true" viewBox="0 0 32 32" width="30" height="30"><path d="M4 16h23m-10-10 10 10-10 10" fill="none" stroke="currentColor" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;

fn page(title: &str, content: &str, rate_limited: bool) -> Html<String> {
    let flash = if rate_limited {
        "<turbo-frame id=\"flash\"><div class=\"flash\"><div class=\"flash__inner shadow\">Try again later.</div></div></turbo-frame>"
    } else {
        "<turbo-frame id=\"flash\"></turbo-frame>"
    };
    Html(format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover"><meta name="color-scheme" content="light dark"><title>{} | Rizzy</title><style>{}</style></head><body class="public"><div id="global-container"><header class="header" id="header"><a href="#main" class="header__skip-navigation btn">Skip to main content</a><nav><a href="/" class="header__logo" aria-label="Rizzy home"><span class="logo-mark" aria-hidden="true"><i></i><i></i><i></i><i></i></span><strong>Rizzy</strong></a></nav></header>{flash}<main id="main"><div class="panel panel--centered flex flex-column gap-half"><h1 class="txt-x-large font-weight-black margin-block-end">{}</h1>{content}</div></main></div><footer id="footer"><div class="pad-inline txt-align-center center margin-block-double txt-subtle txt-small"><a href="https://github.com/basecamp/fizzy" class="txt-current font-weight-bold txt-nowrap" target="_blank" rel="noopener noreferrer">Fizzy™</a> inspired this independent Rust version, Rizzy™. Fizzy is made by <a href="https://37signals.com" class="txt-current font-weight-bold txt-nowrap" target="_blank" rel="noopener noreferrer">37signals™</a>, makers of <a href="https://www.basecamp.com" class="txt-current font-weight-bold txt-nowrap" target="_blank" rel="noopener noreferrer">Basecamp™</a> and <a href="https://www.hey.com" class="txt-current font-weight-bold txt-nowrap" target="_blank" rel="noopener noreferrer">HEY™</a>.</div></footer></body></html>"##,
        escape(title),
        include_str!("../static/signup.css"),
        escape(title),
    ))
}

pub(super) fn new(rate_limited: bool) -> Html<String> {
    page(
        "Sign up",
        &format!(
            r#"<form method="post" action="/signup" class="flex flex-column gap"><div class="flex align-center gap"><label class="flex align-center gap input input--actor"><input type="email" name="signup[email_address]" class="input txt-large full-width" autocomplete="username" placeholder="Enter your email address…" required autofocus></label></div><p>Enter your email to create an account.</p><button id="log_in" type="submit" class="btn btn--link center txt-medium"><span>Let’s go</span>{ARROW}</button></form>"#,
        ),
        rate_limited,
    )
}

pub(super) fn completion(error: Option<&str>) -> Html<String> {
    let error = error.map(|value|format!("<div class=\"margin-block-half txt-negative txt-small error\" style=\"text-align: left;\"><p class=\"margin-block-none font-weight-bold\">Your changes couldn't be saved:</p><ul class=\"margin-block-none\"><li>{}</li></ul></div>",escape(value))).unwrap_or_default();
    page(
        "Complete your sign-up",
        &format!(
            r#"<form method="post" action="/signup/completion" class="flex flex-column gap"><input type="text" name="signup[full_name]" class="input txt-large" autocomplete="name" placeholder="Enter your full name…" maxlength="240" required autofocus><p>You're one step away. Just enter your name to get your own Rizzy account.</p>{error}<button type="submit" class="btn btn--link center"><span>Continue</span>{ARROW}</button></form><p class="import-link"><a href="/account/imports/new" class="btn btn--plain txt-link center txt-small">Or import a Fizzy account</a></p>"#,
        ),
        false,
    )
}
