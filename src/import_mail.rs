use super::*;

fn render_completed(name: &str, url: &str, support: &str) -> (String, String) {
    let plain = format!("Your import of {name} is complete!\n\nHere's the URL: {url}\n\n");
    let content = format!(
        "<p class=\"subtitle\">Your import of {} is complete!</p>\n\n<p><a href=\"{}\">Go to your account</a></p>\n\n<p class=\"footer\">Need help? <a href=\"mailto:{}\">Send us an email</a>.</p>\n",
        escape(name),
        escape(url),
        escape(support)
    );
    let html =
        include_str!("../assets/notification_mail_layout.html").replace("{{content}}", &content);
    (plain, html)
}

fn render_failed(reason: Option<&str>, support: &str) -> (String, String) {
    let detail = match reason {
        Some("conflict") => "It looks like the account you are trying to import already exists.",
        Some("invalid_export") => "The ZIP file isn't a Rizzy account export.",
        _ => {
            "This may be due to corrupted export data or a conflict with existing data. Please try again with a fresh export, or reach out for help if the problem persists."
        }
    };
    let subtitle = "Unfortunately, we couldn't import your Rizzy account.";
    let plain = format!("{subtitle}\n\n{detail}\n\nNeed help? Send us an email at {support}\n\n");
    let content = format!(
        "<p class=\"subtitle\">{subtitle}</p>\n\n  <p>{detail}</p>\n\n<p class=\"footer\">Need help? <a href=\"mailto:{}\">Send us an email</a>.</p>\n",
        escape(support)
    );
    let html =
        include_str!("../assets/notification_mail_layout.html").replace("{{content}}", &content);
    (plain, html)
}

pub(super) async fn send(
    app: &App,
    import_id: &str,
    completed: bool,
    reason: Option<&str>,
) -> Result<()> {
    if env::var("RIZZY_SENDMAIL").is_err() {
        return Ok(());
    }
    let row = sqlx::query("SELECT i.email_address,a.name,a.external_account_id FROM imports m JOIN identities i ON i.id=m.identity_id JOIN accounts a ON a.id=m.account_id WHERE m.id=?")
        .bind(import_id).fetch_one(&app.db).await.map_err(db_err)?;
    let email: String = row.get("email_address");
    let support = mail_transport::support_address()?;
    let (subject, plain, html) = if completed {
        let name: String = row.get("name");
        let slug: i64 = row.get("external_account_id");
        let origin =
            env::var("RIZZY_PUBLIC_URL").unwrap_or_else(|_| "http://localhost:3007".into());
        let url = format!("{}/{slug}/landing", origin.trim_end_matches('/'));
        let (plain, html) = render_completed(&name, &url, &support);
        ("Your Rizzy account import is done", plain, html)
    } else {
        let (plain, html) = render_failed(reason, &support);
        ("Your Rizzy account import failed", plain, html)
    };
    mail_transport::send_multipart(&email, subject, &plain, &html).await
}
