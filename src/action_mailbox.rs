use axum::{
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};

pub(super) async fn unavailable_ingress() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "text/html")],
        "",
    )
        .into_response()
}

pub(super) async fn gated_conductor() -> Response {
    (
        StatusCode::FORBIDDEN,
        [(header::CONTENT_TYPE, "text/html")],
        "",
    )
        .into_response()
}

pub(super) async fn rejected_conductor_write(headers: HeaderMap) -> Response {
    if !headers.contains_key("x-rizzy-json-request")
        && headers
            .get(header::ACCEPT)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value.contains("text/html"))
    {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            [
                (header::CONTENT_TYPE, "text/html; charset=UTF-8"),
                (header::CONTENT_SECURITY_POLICY, super::STATIC_CSP),
            ],
            include_str!("../static/422.html"),
        )
            .into_response();
    }
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        [(header::CONTENT_TYPE, "application/json; charset=UTF-8")],
        r#"{"status":422,"error":"Unprocessable Content"}"#,
    )
        .into_response()
}
