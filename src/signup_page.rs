use axum::response::Html;

use super::escape;

const ARROW: &str = r#"<svg aria-hidden="true" viewBox="0 0 32 32" width="30" height="30"><path d="M4 16h23m-10-10 10 10-10 10" fill="none" stroke="currentColor" stroke-width="4" stroke-linecap="round" stroke-linejoin="round"/></svg>"#;

fn page(
    title: &str,
    heading: &str,
    header_detail: Option<&str>,
    content: &str,
    flash_message: Option<&str>,
    shake: bool,
) -> Html<String> {
    let flash = if let Some(message) = flash_message {
        format!(
            "<turbo-frame id=\"flash\"><div class=\"flash\"><div class=\"flash__inner shadow\">{}</div></div></turbo-frame>",
            escape(message)
        )
    } else {
        "<turbo-frame id=\"flash\"></turbo-frame>".to_owned()
    };
    let shake_class = if shake { " shake" } else { "" };
    let heading = format!(
        "<h1 class=\"txt-x-large font-weight-black margin-block-end\">{}</h1>",
        escape(heading)
    );
    let heading = match header_detail {
        Some(detail) => format!("<header class=\"auth-heading\">{heading}{detail}</header>"),
        None => heading,
    };
    Html(format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1, viewport-fit=cover"><meta name="color-scheme" content="light dark"><title>{} | Rizzy</title><style>{}</style></head><body class="public"><div id="global-container"><header class="header" id="header"><a href="#main" class="header__skip-navigation btn">Skip to main content</a><nav><a href="/" class="header__logo" aria-label="Rizzy home"><span class="logo-mark" aria-hidden="true"><i></i><i></i><i></i><i></i></span><strong>Rizzy</strong></a></nav></header>{flash}<main id="main"><div class="panel panel--centered flex flex-column gap-half{shake_class}">{heading}{content}</div></main></div><footer id="footer"><div class="pad-inline txt-align-center center margin-block-double txt-subtle txt-small"><a href="https://github.com/basecamp/fizzy" class="txt-current font-weight-bold txt-nowrap" target="_blank" rel="noopener noreferrer">Fizzy™</a> inspired this independent Rust version, Rizzy™. Fizzy is made by <a href="https://37signals.com" class="txt-current font-weight-bold txt-nowrap" target="_blank" rel="noopener noreferrer">37signals™</a>, makers of <a href="https://www.basecamp.com" class="txt-current font-weight-bold txt-nowrap" target="_blank" rel="noopener noreferrer">Basecamp™</a> and <a href="https://www.hey.com" class="txt-current font-weight-bold txt-nowrap" target="_blank" rel="noopener noreferrer">HEY™</a>.</div></footer></body></html>"##,
        escape(title),
        include_str!("../static/signup.css"),
    ))
}

pub(super) fn new(rate_limited: bool) -> Html<String> {
    page(
        "Sign up",
        "Sign up",
        None,
        &format!(
            r#"<form method="post" action="/signup" class="flex flex-column gap"><div class="flex align-center gap"><label class="flex align-center gap input input--actor"><input type="email" name="signup[email_address]" class="input txt-large full-width" autocomplete="username" placeholder="Enter your email address…" required autofocus></label></div><p>Enter your email to create an account.</p><button id="log_in" type="submit" class="btn btn--link center txt-medium"><span>Let’s go</span>{ARROW}</button></form>"#,
        ),
        rate_limited.then_some("Try again later."),
        false,
    )
}

pub(super) fn completion(error: Option<&str>) -> Html<String> {
    let error = error.map(|value|format!("<div class=\"margin-block-half txt-negative txt-small error\" style=\"text-align: left;\"><p class=\"margin-block-none font-weight-bold\">Your changes couldn't be saved:</p><ul class=\"margin-block-none\"><li>{}</li></ul></div>",escape(value))).unwrap_or_default();
    page(
        "Complete your sign-up",
        "Complete your sign-up",
        None,
        &format!(
            r#"<form method="post" action="/signup/completion" class="flex flex-column gap"><input type="text" name="signup[full_name]" class="input txt-large" autocomplete="name" placeholder="Enter your full name…" maxlength="240" required autofocus><p>You're one step away. Just enter your name to get your own Rizzy account.</p>{error}<button type="submit" class="btn btn--link center"><span>Continue</span>{ARROW}</button></form><p class="import-link"><a href="/account/imports/new" class="btn btn--plain txt-link center txt-small">Or import a Fizzy account</a></p>"#,
        ),
        None,
        false,
    )
}

pub(super) fn session_new(
    signup_open: bool,
    passkey_button: &str,
    flash_message: Option<&str>,
) -> Html<String> {
    let description = if signup_open {
        "<p><strong>New here?</strong> <a href=\"/signup/new\">Sign up</a> to create an account. <strong>Already have an account?</strong> Enter your email and we’ll get you signed in.</p>"
    } else {
        "<p>Enter your email and we’ll get you signed in.</p>"
    };
    page(
        "Enter your email",
        "Get into Rizzy",
        None,
        &format!(
            r#"<form method="post" action="/session" class="flex flex-column gap-half txt-medium"><div class="flex align-center gap"><label class="flex align-center gap input input--actor"><input type="email" name="email_address" class="input txt-large full-width" autocomplete="username webauthn" placeholder="Enter your email address…" required autofocus></label></div>{description}<button id="log_in" type="submit" class="btn btn--link center txt-medium"><span>Let’s go</span>{ARROW}</button></form><div class="passkey-signin">{passkey_button}</div>"#,
        ),
        flash_message,
        false,
    )
}

pub(super) fn magic_link(email: &str, code_hint: &str, alert: bool, shake: bool) -> Html<String> {
    let email = escape(email);
    page(
        "Check your email",
        "Check your email",
        Some(
            "<p class=\"magic-link-instruction\">Then enter the verification code included in the email below:</p>",
        ),
        &format!(
            r#"<form method="post" action="/session/magic_link" class="magic-link-form"><input name="code" class="input txt-large" autocomplete="one-time-code" maxlength="6" placeholder="••••••" autocorrect="off" autocapitalize="off" spellcheck="false" required autofocus><button type="submit" class="visually-hidden">Continue</button></form><p class="magic-link-expiry">The code sent to <strong>{email}</strong> will work for 15 minutes.</p>{code_hint}<script>const magicForm=document.querySelector('.magic-link-form');const magicInput=magicForm.elements.code;function submitMagicCode(){{if(magicInput.disabled)return;magicForm.requestSubmit();magicInput.disabled=true}}magicInput.addEventListener('keydown',event=>{{if(event.key==='Enter'){{event.preventDefault();submitMagicCode()}}}});magicInput.addEventListener('paste',()=>setTimeout(submitMagicCode,0));</script>"#,
        ),
        alert.then_some("Try again in 15 minutes."),
        shake,
    )
}
