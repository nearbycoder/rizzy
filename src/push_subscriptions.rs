use super::*;

pub(super) async fn delivery_address(raw: &str) -> Result<(String, SocketAddr)> {
    const HOSTS: &[&str] = &[
        "jmt17.google.com",
        "fcm.googleapis.com",
        "updates.push.services.mozilla.com",
        "web.push.apple.com",
        "notify.windows.com",
    ];
    let parsed = url::Url::parse(raw).map_err(|_| invalid_endpoint())?;
    if parsed.scheme() != "https" || parsed.username() != "" || parsed.password().is_some() {
        return Err(invalid_endpoint());
    }
    let host = parsed
        .host_str()
        .ok_or_else(invalid_endpoint)?
        .to_lowercase();
    if !HOSTS
        .iter()
        .any(|permitted| host == *permitted || host.ends_with(&format!(".{permitted}")))
    {
        return Err(invalid_endpoint());
    }
    let port = parsed
        .port_or_known_default()
        .ok_or_else(invalid_endpoint)?;
    let addresses =
        tokio::time::timeout(Duration::from_secs(5), lookup_host((host.as_str(), port)))
            .await
            .map_err(|_| invalid_endpoint())?
            .map_err(|_| invalid_endpoint())?
            .collect::<Vec<_>>();
    if addresses.is_empty() || addresses.iter().any(|address| !public_ip(address.ip())) {
        return Err(invalid_endpoint());
    }
    Ok((host, addresses[0]))
}

async fn validated_endpoint(raw: &str) -> Result<String> {
    delivery_address(raw).await?;
    Ok(raw.to_owned())
}

fn invalid_endpoint() -> ApiError {
    err(
        StatusCode::UNPROCESSABLE_ENTITY,
        "Invalid push subscription endpoint",
    )
}

fn input(headers: &HeaderMap, body: &[u8]) -> Result<Value> {
    if headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"))
    {
        let mut input = serde_json::Map::new();
        for (key, value) in url::form_urlencoded::parse(body) {
            if let Some(name) = key
                .strip_prefix("push_subscription[")
                .and_then(|key| key.strip_suffix(']'))
            {
                input.insert(name.into(), json!(value.into_owned()));
            }
        }
        Ok(Value::Object(input))
    } else {
        let value: Value = serde_json::from_slice(body).map_err(|_| bad("Invalid JSON"))?;
        Ok(nested(&value, "push_subscription").clone())
    }
}

pub(super) async fn index(
    State(app): Shared,
    Path((account, _user)): Path<(String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    principal(&app, &headers, &account).await?;
    Ok((
        StatusCode::NO_CONTENT,
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
    )
        .into_response())
}

pub(super) async fn create(
    State(app): Shared,
    Path((account, _user)): Path<(String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    let input = input(&headers, &body)?;
    let endpoint = validated_endpoint(&required(&input, "endpoint")?).await?;
    let key = string(&input, "p256dh_key");
    let auth = string(&input, "auth_key");
    let timestamp = now();
    let statement = if app.mysql {
        "INSERT IGNORE INTO push_subscriptions(id,account_id,user_id,endpoint,p256dh_key,auth_key,user_agent,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)"
    } else {
        "INSERT OR IGNORE INTO push_subscriptions(id,account_id,user_id,endpoint,p256dh_key,auth_key,user_agent,created_at,updated_at) VALUES(?,?,?,?,?,?,?,?,?)"
    };
    sqlx::query(statement)
        .bind(id())
        .bind(&p.account_id)
        .bind(&p.user_id)
        .bind(endpoint)
        .bind(key)
        .bind(auth)
        .bind(
            headers
                .get(header::USER_AGENT)
                .and_then(|value| value.to_str().ok())
                .map(|value| value.chars().take(4096).collect::<String>()),
        )
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok(if headers.contains_key("x-rizzy-json-request") {
        StatusCode::CREATED
    } else {
        StatusCode::NO_CONTENT
    })
}

pub(super) async fn destroy(
    State(app): Shared,
    Path((account, _user, subscription)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Result<Response> {
    let p = principal(&app, &headers, &account).await?;
    p.write()?;
    sqlx::query("DELETE FROM push_subscriptions WHERE id=? AND user_id=?")
        .bind(subscription)
        .bind(&p.user_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    if headers.contains_key("x-rizzy-json-request") {
        Ok(StatusCode::NO_CONTENT.into_response())
    } else {
        Ok((
            StatusCode::FOUND,
            [(
                header::LOCATION,
                format!("/{}/users/{}/push_subscriptions", p.slug, p.user_id),
            )],
        )
            .into_response())
    }
}

pub(super) async fn form_destroy(
    state: Shared,
    path: Path<(String, String, String)>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    let method = url::form_urlencoded::parse(&body)
        .find(|(key, _)| key == "_method")
        .map(|(_, value)| value.into_owned());
    if method.as_deref() == Some("delete") {
        destroy(state, path, headers).await
    } else {
        Err(missing())
    }
}
