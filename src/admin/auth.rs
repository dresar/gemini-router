use axum::http::HeaderMap;
use subtle::ConstantTimeEq;

pub fn verify_admin_key(expected: &str, provided: &str) -> bool {
    let expected_bytes = expected.as_bytes();
    let provided_bytes = provided.as_bytes();

    if expected_bytes.len() != provided_bytes.len() {
        return false;
    }

    expected_bytes.ct_eq(provided_bytes).into()
}

pub fn extract_token(headers: &HeaderMap) -> Option<&str> {
    if let Some(auth_val) = headers.get("authorization").and_then(|h| h.to_str().ok()) {
        if let Some(token) = auth_val.strip_prefix("Bearer ") {
            return Some(token.trim());
        }
        return Some(auth_val.trim());
    }

    if let Some(key_val) = headers.get("x-admin-key").and_then(|h| h.to_str().ok()) {
        return Some(key_val.trim());
    }

    None
}

pub fn authenticate(headers: &HeaderMap, expected_admin_key: &str) -> bool {
    match extract_token(headers) {
        Some(token) => verify_admin_key(expected_admin_key, token),
        None => false,
    }
}
