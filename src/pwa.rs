use super::*;
use axum::extract::OriginalUri;

pub(super) async fn service_worker(State(app): Shared, headers: HeaderMap) -> Result<Response> {
    identity_auth(&app, &headers).await?;
    let mut script = include_str!("../static/pwa/service-worker.js").to_owned();
    if env::var("SERVICE_WORKER_CORS_ENABLED").as_deref() == Ok("false") {
        script = script.replace("    fetchOptions: { mode: \"cors\" },\n", "");
    }
    Ok((
        [
            (header::CONTENT_TYPE, "text/javascript; charset=utf-8"),
            (header::CACHE_CONTROL, "no-cache"),
        ],
        script,
    )
        .into_response())
}

pub(super) async fn manifest() -> Json<Value> {
    Json(json!({
        "name": "Rizzy",
        "icons": [
            {"src":"/app-icon-192.png","type":"image/png","sizes":"192x192"},
            {"src":"/app-icon.png","type":"image/png","sizes":"512x512"},
            {"src":"/app-icon-192-maskable.png","type":"image/png","sizes":"192x192","purpose":"maskable"}
        ],
        "start_url":"/",
        "display":"standalone",
        "scope":"/",
        "description":"Card-up the biggest issues in your projects",
        "categories":["bugs","business","productivity"],
        "shortcuts":[
            {"name":"Notifications","description":"Catch up on recent notifications","url":"/notifications","icons":[{"src":"/assets/pwa/bell.svg","sizes":"any"}]},
            {"name":"Latest Activity","description":"See what’s new","url":"/","icons":[{"src":"/assets/pwa/activity.svg","sizes":"any"}]}
        ]
    }))
}

pub(super) async fn asset(Path(filename): Path<String>) -> Result<Response> {
    let (content_type, body): (&str, &'static [u8]) = match filename.as_str() {
        "turbo-offline-umd.min.js" => (
            "text/javascript; charset=utf-8",
            include_bytes!("../static/pwa/turbo-offline-umd.min.js"),
        ),
        "bell.svg" => ("image/svg+xml", include_bytes!("../static/pwa/bell.svg")),
        "activity.svg" => (
            "image/svg+xml",
            include_bytes!("../static/pwa/activity.svg"),
        ),
        _ => return Err(missing()),
    };
    Ok((
        [(header::CONTENT_TYPE, content_type)],
        Bytes::from_static(body),
    )
        .into_response())
}

pub(super) async fn icon(OriginalUri(uri): OriginalUri) -> Result<Response> {
    let body: &'static [u8] = match uri.path().trim_start_matches('/') {
        "app-icon.png" => include_bytes!("../static/pwa/app-icon.png"),
        "app-icon-192.png" => include_bytes!("../static/pwa/app-icon-192.png"),
        "app-icon-192-maskable.png" => include_bytes!("../static/pwa/app-icon-192-maskable.png"),
        _ => return Err(missing()),
    };
    Ok((
        [(header::CONTENT_TYPE, "image/png")],
        Bytes::from_static(body),
    )
        .into_response())
}
