use super::*;

pub(super) fn support_address() -> Result<String> {
    let from = env::var("RIZZY_MAIL_FROM").unwrap_or_else(|_| "no-reply@localhost".into());
    let support = env::var("RIZZY_SUPPORT_EMAIL").unwrap_or_else(|_| {
        from.rsplit_once('<')
            .map(|(_, address)| address.trim_end_matches('>').trim().to_owned())
            .unwrap_or(from)
    });
    if support.contains(['\r', '\n', ' ', '<', '>'])
        || support
            .split_once('@')
            .is_none_or(|(local, domain)| local.is_empty() || domain.is_empty())
    {
        return Err(bad("invalid email address"));
    }
    Ok(support)
}

pub(super) async fn send_multipart(
    email: &str,
    subject: &str,
    plain: &str,
    html: &str,
) -> Result<()> {
    let path = env::var("RIZZY_SENDMAIL").map_err(|_| {
        err(
            StatusCode::SERVICE_UNAVAILABLE,
            "Mail delivery is not configured",
        )
    })?;
    let from = env::var("RIZZY_MAIL_FROM").unwrap_or_else(|_| "no-reply@localhost".into());
    if !valid_email(email) || from.contains(['\r', '\n']) || subject.contains(['\r', '\n']) {
        return Err(bad("invalid email address"));
    }
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
