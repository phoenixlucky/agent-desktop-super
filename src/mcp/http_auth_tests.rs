use super::{RequestRejection, generate_token, validate_request};
use axum::http::{HeaderMap, HeaderValue};

const TOKEN: &str = "d9d14f1a752b2e334029e690e15a94847d29d50fb62ea13d4acb46fb56ad65";

fn headers(auth: Option<&str>, host: Option<&str>, origin: Option<&str>) -> HeaderMap {
    let mut headers = HeaderMap::new();
    if let Some(value) = auth {
        headers.insert(
            "authorization",
            HeaderValue::from_str(value).expect("valid auth header"),
        );
    }
    if let Some(value) = host {
        headers.insert(
            "host",
            HeaderValue::from_str(value).expect("valid host header"),
        );
    }
    if let Some(value) = origin {
        headers.insert(
            "origin",
            HeaderValue::from_str(value).expect("valid origin header"),
        );
    }
    headers
}

#[test]
fn bearer_credentials_must_match_the_active_token() {
    let host = Some("127.0.0.1:12899");
    let valid = headers(Some(&format!("Bearer {TOKEN}")), host, None);
    assert_eq!(validate_request(&valid, TOKEN), Ok(()));
    for auth in [None, Some("Basic abc"), Some("Bearer old-token")] {
        assert_eq!(
            validate_request(&headers(auth, host, None), TOKEN),
            Err(RequestRejection::Unauthorized)
        );
    }
}

#[test]
fn host_must_match_the_fixed_loopback_endpoint() {
    assert_eq!(
        validate_request(
            &headers(Some(&format!("Bearer {TOKEN}")), Some("evil.example"), None),
            TOKEN
        ),
        Err(RequestRejection::Forbidden)
    );
    assert_eq!(
        validate_request(
            &headers(Some(&format!("Bearer {TOKEN}")), None, None),
            TOKEN
        ),
        Err(RequestRejection::Forbidden)
    );
}

#[test]
fn only_the_local_endpoint_origin_is_allowed() {
    let auth = format!("Bearer {TOKEN}");
    let host = Some("127.0.0.1:12899");
    for origin in ["https://evil.example", "null", "http://localhost:12899"] {
        assert_eq!(
            validate_request(&headers(Some(auth.as_str()), host, Some(origin)), TOKEN),
            Err(RequestRejection::Forbidden)
        );
    }
    assert_eq!(
        validate_request(
            &headers(Some(auth.as_str()), host, Some("http://127.0.0.1:12899")),
            TOKEN
        ),
        Ok(())
    );
}

#[test]
fn service_token_is_a_fresh_url_safe_256_bit_secret() {
    let first = generate_token().expect("OS random token");
    let second = generate_token().expect("OS random token");
    assert_eq!(first.len(), 43);
    assert!(
        first
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    );
    assert_ne!(first, second);
}
