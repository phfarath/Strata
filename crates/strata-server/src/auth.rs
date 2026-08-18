use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct AuthQuery {
    pub token: Option<String>,
}

/// Extract and validate Bearer authentication token from HTTP headers or query parameter.
pub fn validate_auth(
    headers: &HeaderMap,
    query: Option<&AuthQuery>,
    expected_token: Option<&str>,
) -> Result<(), Response> {
    let secret = match expected_token {
        Some(s) if !s.trim().is_empty() => s.trim(),
        _ => return Ok(()), // Open / development mode: no auth enforced
    };

    // 1. Check Authorization header: `Bearer <token>`
    if let Some(auth_header) = headers.get(axum::http::header::AUTHORIZATION) {
        if let Ok(auth_str) = auth_header.to_str() {
            if let Some(token) = auth_str.strip_prefix("Bearer ").or_else(|| auth_str.strip_prefix("bearer ")) {
                if token.trim() == secret {
                    return Ok(());
                }
            }
        }
    }

    // 2. Check X-Strata-Token or X-Cortex-Key header
    if let Some(x_token) = headers.get("x-strata-token").or_else(|| headers.get("x-cortex-key")) {
        if let Ok(token_str) = x_token.to_str() {
            if token_str.trim() == secret {
                return Ok(());
            }
        }
    }

    // 3. Fallback: Query parameter `?token=<token>` (useful for WebSockets and quick testing)
    if let Some(q) = query {
        if let Some(ref t) = q.token {
            if t.trim() == secret {
                return Ok(());
            }
        }
    }

    // Unauthorized response
    let error_body = serde_json::json!({
        "error": "Unauthorized: invalid or missing authentication token",
        "status": 401
    });

    Err((StatusCode::UNAUTHORIZED, Json(error_body)).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn test_auth_validation_open_mode() {
        let headers = HeaderMap::new();
        assert!(validate_auth(&headers, None, None).is_ok());
        assert!(validate_auth(&headers, None, Some("")).is_ok());
    }

    #[test]
    fn test_auth_validation_bearer_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer my-secret-token"),
        );

        assert!(validate_auth(&headers, None, Some("my-secret-token")).is_ok());
        assert!(validate_auth(&headers, None, Some("wrong-token")).is_err());
    }

    #[test]
    fn test_auth_validation_custom_headers() {
        let mut headers1 = HeaderMap::new();
        headers1.insert("x-strata-token", HeaderValue::from_static("custom-token"));
        assert!(validate_auth(&headers1, None, Some("custom-token")).is_ok());

        let mut headers2 = HeaderMap::new();
        headers2.insert("x-cortex-key", HeaderValue::from_static("cortex-key"));
        assert!(validate_auth(&headers2, None, Some("cortex-key")).is_ok());
    }

    #[test]
    fn test_auth_validation_query_param() {
        let headers = HeaderMap::new();
        let query = AuthQuery {
            token: Some("query-token".to_string()),
        };
        assert!(validate_auth(&headers, Some(&query), Some("query-token")).is_ok());
        assert!(validate_auth(&headers, Some(&query), Some("other-token")).is_err());
    }
}

