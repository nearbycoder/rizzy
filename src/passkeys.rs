use super::*;
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use webauthn_rs::prelude::{
    DiscoverableAuthentication, DiscoverableKey, Passkey, PasskeyRegistration, PublicKeyCredential,
    RegisterPublicKeyCredential, Uuid, Webauthn, WebauthnBuilder,
};

fn webauthn() -> Result<Webauthn> {
    let origin = env::var("RIZZY_PASSKEY_ORIGIN")
        .or_else(|_| env::var("RIZZY_PUBLIC_URL"))
        .unwrap_or_else(|_| "http://localhost:3007".into());
    let origin = url::Url::parse(&origin).map_err(|_| bad("Invalid passkey origin"))?;
    let rp_id = origin
        .host_str()
        .ok_or_else(|| bad("Invalid passkey origin"))?;
    WebauthnBuilder::new(rp_id, &origin)
        .and_then(|builder| builder.rp_name("Rizzy").build())
        .map_err(|_| {
            err(
                StatusCode::SERVICE_UNAVAILABLE,
                "Passkeys require a configured domain origin",
            )
        })
}

fn user_uuid(identity_id: &str) -> Uuid {
    let hash = Sha256::digest(identity_id.as_bytes());
    Uuid::from_bytes(hash[..16].try_into().expect("SHA-256 has 16 bytes"))
}

async fn identity_name(app: &App, identity_id: &str) -> Result<(String, String)> {
    let row = sqlx::query("SELECT i.email_address,COALESCE((SELECT u.name FROM users u WHERE u.identity_id=i.id AND u.active=1 ORDER BY u.created_at LIMIT 1),i.email_address) name FROM identities i WHERE i.id=?")
        .bind(identity_id).fetch_optional(&app.db).await.map_err(db_err)?.ok_or_else(missing)?;
    Ok((row.get("email_address"), row.get("name")))
}

fn challenge_from_options(options: &Value) -> Result<String> {
    options
        .get("publicKey")
        .and_then(|key| key.get("challenge"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .ok_or_else(|| bad("Invalid passkey challenge"))
}

fn fields(headers: &HeaderMap, body: &[u8]) -> Result<Value> {
    let form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"));
    if form {
        let mut value = serde_json::Map::new();
        let mut transports = Vec::new();
        for (key, val) in url::form_urlencoded::parse(body) {
            if key == "passkey[transports][]" {
                if !val.is_empty() {
                    transports.push(val.into_owned());
                }
            } else if let Some(name) = key
                .strip_prefix("passkey[")
                .and_then(|key| key.strip_suffix(']'))
            {
                value.insert(name.into(), json!(val.into_owned()));
            }
        }
        value.insert("transports".into(), json!(transports));
        Ok(Value::Object(value))
    } else {
        let value: Value = serde_json::from_slice(body).map_err(|_| bad("Invalid JSON"))?;
        Ok(nested(&value, "passkey").clone())
    }
}

fn client_challenge(client_data: &str) -> Result<String> {
    let data: Value = serde_json::from_str(client_data).map_err(|_| bad("Invalid client data"))?;
    required(&data, "challenge")
}

async fn consume_challenge(app: &App, challenge: &str, purpose: &str) -> Result<DbRow> {
    if app.mysql {
        let mut tx = app.db.begin().await.map_err(db_err)?;
        let row = sqlx::query("SELECT identity_id,state_json FROM passkey_challenges WHERE challenge=? AND purpose=? AND expires_at>? FOR UPDATE")
            .bind(challenge)
            .bind(purpose)
            .bind(now())
            .fetch_optional(&mut *tx)
            .await
            .map_err(db_err)?
            .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Passkey challenge expired or already used"))?;
        sqlx::query("DELETE FROM passkey_challenges WHERE challenge=? AND purpose=?")
            .bind(challenge)
            .bind(purpose)
            .execute(&mut *tx)
            .await
            .map_err(db_err)?;
        tx.commit().await.map_err(db_err)?;
        Ok(row)
    } else {
        sqlx::query("DELETE FROM passkey_challenges WHERE challenge=? AND purpose=? AND expires_at>? RETURNING identity_id,state_json")
            .bind(challenge)
            .bind(purpose)
            .bind(now())
            .fetch_optional(&app.db)
            .await
            .map_err(db_err)?
            .ok_or_else(|| err(StatusCode::UNAUTHORIZED, "Passkey challenge expired or already used"))
    }
}

pub(super) async fn challenge(
    State(app): Shared,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Value>> {
    let purpose = if headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"))
    {
        url::form_urlencoded::parse(&body)
            .find(|(key, _)| key == "purpose")
            .map(|(_, val)| val.into_owned())
            .unwrap_or_default()
    } else {
        serde_json::from_slice::<Value>(&body)
            .ok()
            .and_then(|value| string(&value, "purpose"))
            .unwrap_or_default()
    };
    let registration = purpose == "registration";
    let webauthn = webauthn()?;
    let identity = if registration {
        identity_auth(&app, &headers)
            .await
            .ok()
            .map(|(identity, _)| identity)
    } else {
        None
    };
    let (options, state, purpose, expiration) = if registration {
        let user_id = identity
            .as_deref()
            .map(user_uuid)
            .unwrap_or_else(|| Uuid::from_bytes(rand::random::<[u8; 16]>()));
        let (name, display) = if let Some(identity) = &identity {
            identity_name(&app, identity).await?
        } else {
            ("unregistered@example.test".into(), "Unregistered".into())
        };
        let (options, state) = webauthn
            .start_passkey_registration(user_id, &name, &display, None)
            .map_err(|_| bad("Could not create passkey challenge"))?;
        (
            serde_json::to_value(options).map_err(|_| bad("Invalid passkey options"))?,
            serde_json::to_string(&state).map_err(|_| bad("Invalid passkey state"))?,
            "registration",
            10,
        )
    } else {
        let (options, state) = webauthn
            .start_discoverable_authentication()
            .map_err(|_| bad("Could not create passkey challenge"))?;
        (
            serde_json::to_value(options).map_err(|_| bad("Invalid passkey options"))?,
            serde_json::to_string(&state).map_err(|_| bad("Invalid passkey state"))?,
            "authentication",
            5,
        )
    };
    let challenge = challenge_from_options(&options)?;
    let expires_at = (Utc::now() + chrono::Duration::minutes(expiration))
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    sqlx::query("INSERT INTO passkey_challenges(challenge,purpose,identity_id,state_json,expires_at,created_at) VALUES(?,?,?,?,?,?)")
        .bind(&challenge).bind(purpose).bind(identity).bind(state).bind(expires_at).bind(now())
        .execute(&app.db).await.map_err(db_err)?;
    Ok(Json(json!({"challenge":challenge})))
}

fn scope_prefix(uri: &Uri) -> &str {
    uri.path()
        .split_once("/my/passkeys")
        .map_or("", |(prefix, _)| prefix)
}

pub(super) async fn unscoped_redirect() -> Response {
    (
        StatusCode::FOUND,
        [(header::LOCATION, "/session/menu")],
        Html(String::new()),
    )
        .into_response()
}

pub(super) async fn index(
    State(app): Shared,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let (identity_id, _) = identity_auth(&app, &headers).await?;
    let (email, name) = identity_name(&app, &identity_id).await?;
    let webauthn = webauthn()?;
    let keys = sqlx::query("SELECT id,name,created_at FROM passkeys WHERE identity_id=? ORDER BY lower(name),created_at DESC")
        .bind(&identity_id).fetch_all(&app.db).await.map_err(db_err)?;
    let mut items = String::new();
    let prefix = scope_prefix(&uri);
    for key in keys {
        let id: String = key.get("id");
        let title: Option<String> = key.get("name");
        items.push_str(&format!(
            "<li><a href=\"{prefix}/my/passkeys/{}/edit\">{}</a></li>",
            escape(&id),
            escape(title.as_deref().unwrap_or("Passkey"))
        ));
    }
    let (creation, _) = webauthn
        .start_passkey_registration(user_uuid(&identity_id), &email, &name, None)
        .map_err(|_| bad("Could not create passkey options"))?;
    let mut registration_options =
        serde_json::to_value(&creation.public_key).map_err(|_| bad("Invalid passkey options"))?;
    registration_options["authenticatorSelection"]["residentKey"] = json!("required");
    registration_options["authenticatorSelection"]["requireResidentKey"] = json!(true);
    let options = serde_json::to_string(&registration_options)
        .map_err(|_| bad("Invalid passkey options"))?
        .replace('<', "\\u003c");
    let html = format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Passkeys | Rizzy</title><style>:root{{font:16px/1.5 system-ui,sans-serif;background:#1b1b1b;color:#f5f0e8}}main{{max-width:650px;margin:40px auto;padding:24px;background:#292826;border-radius:12px}}a{{color:#e7bd61}}button{{padding:10px 16px;background:#e7bd61;border:0;border-radius:8px;font:inherit;cursor:pointer}}</style></head><body><main><p><a href="/session/menu">← Menu</a></p><h1>Passkeys</h1><p>Passkeys let you sign in securely without a password or email code.</p><ul>{items}</ul><form id="register-form" method="post" action="{prefix}/my/passkeys"><input type="hidden" name="passkey[client_data_json]"><input type="hidden" name="passkey[attestation_object]"><button type="button" id="register">Register a passkey</button></form><p>Your browser will prompt you to create a passkey using your device's biometrics, PIN, or security key</p><p role="alert" id="passkey-error"></p></main><script>const options={options};const decode=s=>Uint8Array.from(atob(s.replace(/-/g,'+').replace(/_/g,'/')+'='.repeat((4-s.length%4)%4)),c=>c.charCodeAt(0));const encode=b=>btoa(String.fromCharCode(...new Uint8Array(b))).replace(/\+/g,'-').replace(/\//g,'_').replace(/=+$/,'');document.getElementById('register').onclick=async()=>{{try{{const response=await fetch('/my/passkey_challenge',{{method:'POST',headers:{{Accept:'application/json','Content-Type':'application/x-www-form-urlencoded'}},body:'purpose=registration'}});if(!response.ok)throw Error('Could not get challenge');const registrationOptions={{...options,challenge:decode((await response.json()).challenge),user:{{...options.user,id:decode(options.user.id)}},excludeCredentials:(options.excludeCredentials||[]).map(c=>({{...c,id:decode(c.id)}}))}};const credential=await navigator.credentials.create({{publicKey:registrationOptions}});const form=document.getElementById('register-form');form.querySelector('[name="passkey[client_data_json]"]').value=new TextDecoder().decode(credential.response.clientDataJSON);form.querySelector('[name="passkey[attestation_object]"]').value=encode(credential.response.attestationObject);for(const transport of credential.response.getTransports?.()||[]){{const field=document.createElement('input');field.type='hidden';field.name='passkey[transports][]';field.value=transport;form.append(field)}}form.submit()}}catch(error){{document.getElementById('passkey-error').textContent=error.message}}}}</script></body></html>"##
    );
    Ok(Html(html).into_response())
}

pub(super) async fn index_scoped(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    principal(&app, &headers, &account).await?;
    index(State(app), headers, OriginalUri(uri)).await
}

pub(super) async fn register(
    State(app): Shared,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response> {
    let (identity_id, permission) = identity_auth(&app, &headers).await?;
    if permission != "write" {
        return Err(err(StatusCode::FORBIDDEN, "Write permission required"));
    }
    let prefix = scope_prefix(&uri);
    let response = || {
        (
            StatusCode::FOUND,
            [(header::LOCATION, format!("{prefix}/my/passkeys"))],
            Html(String::new()),
        )
            .into_response()
    };
    let input = match fields(&headers, &body) {
        Ok(input) => input,
        Err(_) => return Ok(response()),
    };
    let Some(client_data) = string(&input, "client_data_json") else {
        return Ok(response());
    };
    let Some(attestation) = string(&input, "attestation_object") else {
        return Ok(response());
    };
    let challenge = match client_challenge(&client_data) {
        Ok(challenge) => challenge,
        Err(_) => return Ok(response()),
    };
    let record = match consume_challenge(&app, &challenge, "registration").await {
        Ok(record) => record,
        Err(_) => return Ok(response()),
    };
    if record.get::<Option<String>, _>("identity_id").as_deref() != Some(&identity_id) {
        return Ok(response());
    }
    let Ok(state) =
        serde_json::from_str::<PasskeyRegistration>(&record.get::<String, _>("state_json"))
    else {
        return Ok(response());
    };
    let credential: RegisterPublicKeyCredential = match serde_json::from_value(json!({
        "id":"", "rawId":"", "type":"public-key", "response":{
            "clientDataJSON": URL_SAFE_NO_PAD.encode(client_data.as_bytes()),
            "attestationObject":attestation
        }
    })) {
        Ok(credential) => credential,
        Err(_) => return Ok(response()),
    };
    let passkey = match webauthn()?.finish_passkey_registration(&credential, &state) {
        Ok(passkey) => passkey,
        Err(_) => return Ok(response()),
    };
    let id = id();
    let credential_id = URL_SAFE_NO_PAD.encode(passkey.cred_id());
    let credential_json = serde_json::to_string(&passkey).map_err(|_| bad("Invalid passkey"))?;
    let transports = input.get("transports").cloned().unwrap_or(json!([]));
    let timestamp = now();
    let statement = if app.mysql {
        "INSERT IGNORE INTO passkeys(id,identity_id,credential_id,credential_json,transports,created_at,updated_at) VALUES(?,?,?,?,?,?,?)"
    } else {
        "INSERT OR IGNORE INTO passkeys(id,identity_id,credential_id,credential_json,transports,created_at,updated_at) VALUES(?,?,?,?,?,?,?)"
    };
    let inserted = sqlx::query(statement)
        .bind(&id)
        .bind(identity_id)
        .bind(credential_id)
        .bind(credential_json)
        .bind(transports.to_string())
        .bind(&timestamp)
        .bind(&timestamp)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    if inserted.rows_affected() == 0 {
        return Ok(response());
    }
    Ok((
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("{prefix}/my/passkeys/{id}/edit?created=true"),
        )],
        Html(String::new()),
    )
        .into_response())
}

pub(super) async fn register_scoped(
    State(app): Shared,
    Path(account): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response> {
    principal(&app, &headers, &account).await?;
    register(State(app), headers, OriginalUri(uri), body).await
}

fn missing_passkey(headers: &HeaderMap) -> Response {
    if headers.contains_key("x-rizzy-json-request") {
        (
            StatusCode::NOT_FOUND,
            Json(json!({"status":404,"error":"Not Found"})),
        )
            .into_response()
    } else {
        static_error_page(include_str!("../static/404.html"), StatusCode::NOT_FOUND)
    }
}

async fn owned_passkey(app: &App, headers: &HeaderMap, id: &str) -> Result<Option<DbRow>> {
    let (identity_id, _) = identity_auth(app, headers).await?;
    sqlx::query("SELECT * FROM passkeys WHERE id=? AND identity_id=?")
        .bind(id)
        .bind(identity_id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)
}

pub(super) async fn edit(
    State(app): Shared,
    Path(id): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let Some(key) = owned_passkey(&app, &headers, &id).await? else {
        return Ok(missing_passkey(&headers));
    };
    if headers.contains_key("x-rizzy-json-request") {
        return Err(err(StatusCode::NOT_ACCEPTABLE, "Not Acceptable"));
    }
    let name: Option<String> = key.get("name");
    let prefix = scope_prefix(&uri);
    let created_message = if uri.query().is_some_and(|query| {
        url::form_urlencoded::parse(query.as_bytes()).any(|(key, _)| key == "created")
    }) {
        "<p>Your passkey has been registered. Give it a name so you can identify it later.</p>"
    } else {
        ""
    };
    let html = format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>Edit Passkey | Rizzy</title><style>:root{{font:16px/1.5 system-ui,sans-serif;background:#1b1b1b;color:#f5f0e8}}main{{max-width:620px;margin:40px auto;padding:24px;background:#292826;border-radius:12px}}a{{color:#e7bd61}}input{{display:block;width:95%;margin:12px 0;padding:10px;background:#34322f;color:#fff;border:1px solid #666158;border-radius:8px;font:inherit}}button{{padding:10px 16px;background:#e7bd61;border:0;border-radius:8px;font:inherit;cursor:pointer}}</style></head><body><main><p><a href="{prefix}/my/passkeys">← Passkeys</a></p><h1>Edit Passkey</h1>{created_message}<form method="post" action="{prefix}/my/passkeys/{id}"><input type="hidden" name="_method" value="patch"><label for="name">Name your passkey</label><input id="name" name="passkey[name]" value="{}" placeholder="e.g. MacBook Pro, iPhone" autocomplete="off" autofocus><button type="submit">Save</button></form><form method="post" action="{prefix}/my/passkeys/{id}"><input type="hidden" name="_method" value="delete"><button type="submit">Remove this passkey</button></form></main></body></html>"##,
        escape(name.as_deref().unwrap_or_default())
    );
    Ok(Html(html).into_response())
}

pub(super) async fn edit_scoped(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    principal(&app, &headers, &account).await?;
    edit(State(app), Path(id), headers, OriginalUri(uri)).await
}

pub(super) async fn update(
    State(app): Shared,
    Path(id): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response> {
    let (identity_id, permission) = identity_auth(&app, &headers).await?;
    if permission != "write" {
        return Err(err(StatusCode::FORBIDDEN, "Write permission required"));
    }
    if owned_passkey(&app, &headers, &id).await?.is_none() {
        return Ok(missing_passkey(&headers));
    }
    let name = if headers
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.starts_with("application/x-www-form-urlencoded"))
    {
        url::form_urlencoded::parse(&body)
            .find(|(key, _)| key == "passkey[name]")
            .map(|(_, value)| value.into_owned())
            .unwrap_or_default()
    } else {
        string(&fields(&headers, &body)?, "name").unwrap_or_default()
    };
    sqlx::query("UPDATE passkeys SET name=?,updated_at=? WHERE id=? AND identity_id=?")
        .bind(name.trim())
        .bind(now())
        .bind(&id)
        .bind(identity_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok((
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("{}/my/passkeys", scope_prefix(&uri)),
        )],
        Html(String::new()),
    )
        .into_response())
}

pub(super) async fn update_scoped(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response> {
    principal(&app, &headers, &account).await?;
    update(State(app), Path(id), headers, OriginalUri(uri), body).await
}

pub(super) async fn destroy(
    State(app): Shared,
    Path(id): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    let (identity_id, permission) = identity_auth(&app, &headers).await?;
    if permission != "write" {
        return Err(err(StatusCode::FORBIDDEN, "Write permission required"));
    }
    if owned_passkey(&app, &headers, &id).await?.is_none() {
        return Ok(missing_passkey(&headers));
    }
    sqlx::query("DELETE FROM passkeys WHERE id=? AND identity_id=?")
        .bind(id)
        .bind(identity_id)
        .execute(&app.db)
        .await
        .map_err(db_err)?;
    Ok((
        StatusCode::FOUND,
        [(
            header::LOCATION,
            format!("{}/my/passkeys", scope_prefix(&uri)),
        )],
        Html(String::new()),
    )
        .into_response())
}

pub(super) async fn destroy_scoped(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
) -> Result<Response> {
    principal(&app, &headers, &account).await?;
    destroy(State(app), Path(id), headers, OriginalUri(uri)).await
}

pub(super) async fn form_update_or_delete(
    State(app): Shared,
    Path(id): Path<String>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response> {
    let method = url::form_urlencoded::parse(&body)
        .find(|(key, _)| key == "_method")
        .map(|(_, value)| value.into_owned());
    match method.as_deref() {
        Some("patch") | Some("put") => {
            update(State(app), Path(id), headers, OriginalUri(uri), body).await
        }
        Some("delete") => destroy(State(app), Path(id), headers, OriginalUri(uri)).await,
        _ => Err(missing()),
    }
}

pub(super) async fn form_update_or_delete_scoped(
    State(app): Shared,
    Path((account, id)): Path<(String, String)>,
    headers: HeaderMap,
    OriginalUri(uri): OriginalUri,
    body: Bytes,
) -> Result<Response> {
    principal(&app, &headers, &account).await?;
    form_update_or_delete(State(app), Path(id), headers, OriginalUri(uri), body).await
}

fn failed_authentication(json_request: bool) -> Response {
    if json_request {
        (
            StatusCode::UNAUTHORIZED,
            Json(json!({"message":"That passkey didn't work. Try again."})),
        )
            .into_response()
    } else {
        (StatusCode::FOUND, [(header::LOCATION, "/session/new")]).into_response()
    }
}

pub(super) async fn sign_in(
    State(app): Shared,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Response> {
    auth_attempt(
        &app,
        &format!("passkey:{}", peer.ip()),
        10,
        3,
        "Try again later.",
    )
    .await?;
    let json_request = headers.contains_key("x-rizzy-json-request");
    let input = match fields(&headers, &body) {
        Ok(input) => input,
        Err(_) => return Ok(failed_authentication(json_request)),
    };
    let Some(client_data) = string(&input, "client_data_json") else {
        return Ok(failed_authentication(json_request));
    };
    let Some(id) = string(&input, "id") else {
        return Ok(failed_authentication(json_request));
    };
    let Some(auth_data) = string(&input, "authenticator_data") else {
        return Ok(failed_authentication(json_request));
    };
    let Some(signature) = string(&input, "signature") else {
        return Ok(failed_authentication(json_request));
    };
    let challenge = match client_challenge(&client_data) {
        Ok(challenge) => challenge,
        Err(_) => return Ok(failed_authentication(json_request)),
    };
    let state = match consume_challenge(&app, &challenge, "authentication").await {
        Ok(state) => state,
        Err(_) => return Ok(failed_authentication(json_request)),
    };
    let Some(record) = sqlx::query("SELECT * FROM passkeys WHERE credential_id=?")
        .bind(&id)
        .fetch_optional(&app.db)
        .await
        .map_err(db_err)?
    else {
        return Ok(failed_authentication(json_request));
    };
    let credential: PublicKeyCredential = match serde_json::from_value(json!({
        "id":id, "rawId":id, "type":"public-key", "response":{
            "clientDataJSON":URL_SAFE_NO_PAD.encode(client_data.as_bytes()),
            "authenticatorData":auth_data,"signature":signature,"userHandle":null
        }
    })) {
        Ok(credential) => credential,
        Err(_) => return Ok(failed_authentication(json_request)),
    };
    let Ok(state) =
        serde_json::from_str::<DiscoverableAuthentication>(&state.get::<String, _>("state_json"))
    else {
        return Ok(failed_authentication(json_request));
    };
    let Ok(mut passkey) =
        serde_json::from_str::<Passkey>(&record.get::<String, _>("credential_json"))
    else {
        return Ok(failed_authentication(json_request));
    };
    let key = DiscoverableKey::from(&passkey);
    let Ok(result) = webauthn()?.finish_discoverable_authentication(&credential, state, &[key])
    else {
        return Ok(failed_authentication(json_request));
    };
    passkey.update_credential(&result);
    let token = random_token();
    let mut tx = app.db.begin().await.map_err(db_err)?;
    sqlx::query("UPDATE passkeys SET credential_json=?,sign_count=?,updated_at=? WHERE id=?")
        .bind(serde_json::to_string(&passkey).map_err(|_| bad("Invalid passkey"))?)
        .bind(result.counter() as i64)
        .bind(now())
        .bind(record.get::<String, _>("id"))
        .execute(&mut *tx)
        .await
        .map_err(db_err)?;
    sqlx::query(
        "INSERT INTO sessions(id,identity_id,token_hash,user_agent,created_at) VALUES(?,?,?,?,?)",
    )
    .bind(super::id())
    .bind(record.get::<String, _>("identity_id"))
    .bind(hash(&token))
    .bind(
        headers
            .get(header::USER_AGENT)
            .and_then(|value| value.to_str().ok()),
    )
    .bind(now())
    .execute(&mut *tx)
    .await
    .map_err(db_err)?;
    tx.commit().await.map_err(db_err)?;
    let mut response = if json_request {
        Json(json!({"session_token":token})).into_response()
    } else {
        (StatusCode::FOUND, [(header::LOCATION, "/")]).into_response()
    };
    set_cookie(
        response.headers_mut(),
        "session_token",
        &token,
        60 * 60 * 24 * 30,
        secure_request(&headers),
    );
    Ok(response)
}

pub(super) fn sign_in_button() -> Result<String> {
    let (options, _) = webauthn()?
        .start_discoverable_authentication()
        .map_err(|_| bad("Could not create passkey options"))?;
    let options = serde_json::to_string(&options.public_key)
        .map_err(|_| bad("Invalid passkey options"))?
        .replace('<', "\\u003c");
    Ok(format!(
        r##"<section><h2>Or use a passkey</h2><form method="post" action="/session/passkey" id="passkey-form"><input type="hidden" name="passkey[id]"><input type="hidden" name="passkey[client_data_json]"><input type="hidden" name="passkey[authenticator_data]"><input type="hidden" name="passkey[signature]"><button type="button" id="passkey-signin">Sign in with a passkey</button></form><p role="alert" id="passkey-error"></p></section><script>const passkeyOptions={options};const decodePasskey=s=>Uint8Array.from(atob(s.replace(/-/g,'+').replace(/_/g,'/')+'='.repeat((4-s.length%4)%4)),c=>c.charCodeAt(0));const encodePasskey=b=>btoa(String.fromCharCode(...new Uint8Array(b))).replace(/\+/g,'-').replace(/\//g,'_').replace(/=+$/,'');document.getElementById('passkey-signin').onclick=async()=>{{try{{const response=await fetch('/my/passkey_challenge',{{method:'POST',headers:{{Accept:'application/json'}}}});if(!response.ok)throw Error('Could not get challenge');const options={{...passkeyOptions,challenge:decodePasskey((await response.json()).challenge)}};delete options.allowCredentials;const credential=await navigator.credentials.get({{publicKey:options}});const form=document.getElementById('passkey-form');for(const [name,value] of Object.entries({{id:credential.id,client_data_json:new TextDecoder().decode(credential.response.clientDataJSON),authenticator_data:encodePasskey(credential.response.authenticatorData),signature:encodePasskey(credential.response.signature)}}))form.querySelector(`[name="passkey[${{name}}]"]`).value=value;form.submit()}}catch(error){{document.getElementById('passkey-error').textContent=error.message}}}}</script>"##
    ))
}
