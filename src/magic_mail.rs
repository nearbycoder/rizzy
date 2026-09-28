use super::*;

fn render(code: &str, purpose: &str, passkey_url: Option<&str>, support: &str) -> (String, String) {
    let sign_in = purpose == "sign_in";
    let instruction = if sign_in {
        "Please enter this 6-character verification code on the Rizzy sign-in page:"
    } else {
        "Please enter this 6-character verification code to finish creating your account:"
    };
    let mut plain = format!("{instruction}\n\n{code}\n\nThis code will work for 15 minutes.\n\n");
    if let Some(url) = passkey_url {
        plain.push_str(&format!("P.S. If you want to sign-in faster, and make your account more secure, add a passkey: {url}\n\n"));
    } else {
        plain.push('\n');
    }
    let heading = if sign_in {
        "Rizzy verification code"
    } else {
        "Welcome to Rizzy!"
    };
    let passkey = passkey_url
        .map(|url| format!("  <p>P.S. You can make your account more secure and sign-in faster with a <a href=\"{}\">Passkey</a></p>\n", escape(url)))
        .unwrap_or_default();
    let content = format!(
        "\n      \n  <h1 class=\"title\">{heading}</h1>\n  <p class=\"subtitle\">{instruction}</p>\n\n<strong class=\"txt-large\">{}</strong>\n\n<p>This code will work for 15 minutes.</p>\n\n{passkey}\n<p class=\"footer\">Need help? <a href=\"mailto:{}\">Send us an email</a>.\n</p>\n\n\n    ",
        escape(code),
        escape(support)
    );
    let html =
        include_str!("../assets/notification_mail_layout.html").replace("{{content}}", &content);
    (plain, html)
}

pub(super) async fn deliver(
    app: &App,
    identity_id: &str,
    email: &str,
    code: &str,
    purpose: &str,
    host: &str,
) -> Result<()> {
    if super::dev_magic_links() {
        return Ok(());
    }
    let path = env::var("RIZZY_SENDMAIL").map_err(|_| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "Mail delivery is not configured",
        )
    })?;
    let from = env::var("RIZZY_MAIL_FROM").unwrap_or_else(|_| "no-reply@localhost".into());
    let support = env::var("RIZZY_SUPPORT_EMAIL").unwrap_or_else(|_| {
        from.rsplit_once('<')
            .map(|(_, address)| address.trim_end_matches('>').trim().to_owned())
            .unwrap_or_else(|| from.clone())
    });
    if !valid_email(email) || from.contains(['\r', '\n']) || !valid_email(&support) {
        return Err(bad("invalid email address"));
    }
    let account: Option<i64> = sqlx::query_scalar("SELECT a.external_account_id FROM accounts a JOIN users u ON u.account_id=a.id WHERE u.identity_id=? ORDER BY a.created_at DESC,a.id DESC LIMIT 1")
        .bind(identity_id).fetch_optional(&app.db).await.map_err(db_err)?;
    let passkey_url =
        account.map(|slug| format!("{}/{slug}/my/passkeys", host.trim_end_matches('/')));
    let (plain, html) = render(code, purpose, passkey_url.as_deref(), &support);
    let subject = format!("Your Rizzy code is {code}");
    let boundary = format!("rizzy-{}", id());
    let message = format!(
        "From: {from}\r\nTo: {email}\r\nSubject: {subject}\r\nMIME-Version: 1.0\r\nContent-Type: multipart/alternative; boundary=\"{boundary}\"\r\n\r\n--{boundary}\r\nContent-Type: text/plain; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{}\r\n--{boundary}\r\nContent-Type: text/html; charset=UTF-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{}\r\n--{boundary}--\r\n",
        plain.replace('\n', "\r\n"),
        html.replace('\n', "\r\n")
    );
    let mut child = Command::new(path)
        .arg("-t")
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"))?;
    let Some(mut stdin) = child.stdin.take() else {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"));
    };
    stdin
        .write_all(message.as_bytes())
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"))?;
    drop(stdin);
    let status = child
        .wait()
        .await
        .map_err(|_| err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"))?;
    if !status.success() {
        return Err(err(StatusCode::SERVICE_UNAVAILABLE, "Mail delivery failed"));
    }
    Ok(())
}
