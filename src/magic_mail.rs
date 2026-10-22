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
    let support = mail_transport::support_address()?;
    let account: Option<i64> = sqlx::query_scalar("SELECT a.external_account_id FROM accounts a JOIN users u ON u.account_id=a.id WHERE u.identity_id=? ORDER BY a.created_at DESC,a.id DESC LIMIT 1")
        .bind(identity_id).fetch_optional(&app.db).await.map_err(db_err)?;
    let passkey_url =
        account.map(|slug| format!("{}/{slug}/my/passkeys", host.trim_end_matches('/')));
    let (plain, html) = render(code, purpose, passkey_url.as_deref(), &support);
    let subject = format!("Your Rizzy code is {code}");
    mail_transport::send_multipart(email, &subject, &plain, &html).await
}
