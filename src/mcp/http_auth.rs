use axum::{
    http::{HeaderMap, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use subtle::ConstantTimeEq;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RequestRejection {
    Unauthorized,
    Forbidden,
}

pub(super) fn generate_token() -> Result<String, getrandom::Error> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes)?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

pub(super) fn validate_request(headers: &HeaderMap, token: &str) -> Result<(), RequestRejection> {
    if single_header(headers, header::HOST).and_then(|value| value.to_str().ok())
        != Some("127.0.0.1:12899")
    {
        return Err(RequestRejection::Forbidden);
    }
    let origins = headers.get_all(header::ORIGIN).iter().collect::<Vec<_>>();
    if origins.len() > 1
        || origins
            .first()
            .is_some_and(|origin| origin.to_str().ok() != Some("http://127.0.0.1:12899"))
    {
        return Err(RequestRejection::Forbidden);
    }
    let authorization = single_header(headers, header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(RequestRejection::Unauthorized)?;
    if !bool::from(authorization.as_bytes().ct_eq(token.as_bytes())) {
        return Err(RequestRejection::Unauthorized);
    }
    Ok(())
}

pub(super) async fn authorize_request(
    axum::extract::State(token): axum::extract::State<String>,
    request: axum::http::Request<axum::body::Body>,
    next: Next,
) -> Response {
    match validate_request(request.headers(), &token) {
        Ok(()) => next.run(request).await,
        Err(RequestRejection::Unauthorized) => StatusCode::UNAUTHORIZED.into_response(),
        Err(RequestRejection::Forbidden) => StatusCode::FORBIDDEN.into_response(),
    }
}

fn single_header(headers: &HeaderMap, name: header::HeaderName) -> Option<&header::HeaderValue> {
    let mut values = headers.get_all(name).iter();
    let first = values.next()?;
    values.next().is_none().then_some(first)
}

#[cfg(test)]
#[path = "http_auth_tests.rs"]
mod tests;
