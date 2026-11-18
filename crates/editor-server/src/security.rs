//! Protections for a server that is only meant to be used from this computer.
//!
//! * **Host check** — the server listens on 127.0.0.1, but a malicious web page
//!   could still reach it through DNS rebinding (a domain that resolves to
//!   127.0.0.1). Such requests carry the attacker's domain in `Host`, so only
//!   loopback host names are accepted.
//! * **Origin check** — browsers send cross-site `POST`/`PUT`/`DELETE` requests
//!   to localhost; state-changing requests must come from the editor itself.
//! * **Security headers** — a strict Content-Security-Policy (only the app's
//!   own scripts and styles; nothing from the network) and related headers.

use axum::extract::Request;
use axum::http::{HeaderMap, HeaderValue, Method, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use crate::error::ApiError;

const LOOPBACK_HOSTS: [&str; 3] = ["localhost", "127.0.0.1", "[::1]"];

/// Content-Security-Policy for every response.
pub const CONTENT_SECURITY_POLICY: &str = "default-src 'self'; script-src 'self'; style-src 'self'; \
img-src 'self' data: blob:; font-src 'self'; connect-src 'self'; object-src 'none'; base-uri 'none'; \
frame-ancestors 'none'; form-action 'self'";

/// Rejects requests that were not addressed to a loopback host name, and
/// state-changing requests from other origins.
pub async fn local_only(request: Request, next: Next) -> Response {
    let headers = request.headers();
    if !host_is_loopback(headers) {
        return ApiError::forbidden("This editor only accepts requests addressed to localhost.").into_response();
    }
    if !is_safe_method(request.method()) && !same_origin(headers) {
        return ApiError::forbidden("Cross-origin requests are not allowed.").into_response();
    }
    next.run(request).await
}

/// Adds security headers to every response.
pub async fn security_headers(request: Request, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(CONTENT_SECURITY_POLICY));
    headers.insert(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    headers.insert(header::REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    headers.insert("cross-origin-opener-policy", HeaderValue::from_static("same-origin"));
    headers.insert("cross-origin-resource-policy", HeaderValue::from_static("same-origin"));
    response
}

fn is_safe_method(method: &Method) -> bool {
    matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// Reports whether the `Host` header names a loopback host (any port).
pub fn host_is_loopback(headers: &HeaderMap) -> bool {
    headers.get(header::HOST).and_then(|value| value.to_str().ok()).is_some_and(is_loopback_authority)
}

/// `authority` is `host` or `host:port`; IPv6 hosts are bracketed.
fn is_loopback_authority(authority: &str) -> bool {
    let host = match authority.rfind(':') {
        // A colon after the closing bracket (or with no brackets) separates the port.
        Some(idx) if !authority[idx..].contains(']') => {
            let port = &authority[idx + 1..];
            if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
                return false;
            }
            &authority[..idx]
        }
        _ => authority,
    };
    LOOPBACK_HOSTS.iter().any(|allowed| host.eq_ignore_ascii_case(allowed))
}

/// For state-changing requests: the `Origin` header, if present, must be a
/// loopback origin; without it, `Sec-Fetch-Site` must not say cross-site.
/// Non-browser clients (no `Origin`, no `Sec-Fetch-Site`) are allowed: they
/// already have full access to the local machine.
fn same_origin(headers: &HeaderMap) -> bool {
    if let Some(origin) = headers.get(header::ORIGIN) {
        let Ok(origin) = origin.to_str() else { return false };
        return origin
            .strip_prefix("http://")
            .is_some_and(|authority| !authority.contains('/') && is_loopback_authority(authority));
    }
    match headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        Some(site) => site == "same-origin" || site == "none",
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loopback_authorities() {
        for ok in ["localhost", "localhost:8080", "LOCALHOST:1", "127.0.0.1", "127.0.0.1:8080", "[::1]", "[::1]:8080"] {
            assert!(is_loopback_authority(ok), "{ok}");
        }
        for bad in [
            "example.com",
            "evil.localhost",
            "localhost.evil.com",
            "localhost:",
            "localhost:80x",
            "127.0.0.2",
            "0.0.0.0:8080",
            "[::2]:8080",
            "192.168.1.10:8080",
            "",
        ] {
            assert!(!is_loopback_authority(bad), "{bad}");
        }
    }

    fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
        let mut map = HeaderMap::new();
        for (name, value) in pairs {
            map.insert(*name, HeaderValue::from_static(value));
        }
        map
    }

    #[test]
    fn origin_rules() {
        assert!(same_origin(&headers(&[])));
        assert!(same_origin(&headers(&[("origin", "http://localhost:8080")])));
        assert!(same_origin(&headers(&[("origin", "http://127.0.0.1:8080")])));
        assert!(same_origin(&headers(&[("sec-fetch-site", "same-origin")])));
        assert!(!same_origin(&headers(&[("origin", "https://evil.example")])));
        assert!(!same_origin(&headers(&[("origin", "http://localhost.evil.example")])));
        assert!(!same_origin(&headers(&[("origin", "null")])));
        assert!(!same_origin(&headers(&[("sec-fetch-site", "cross-site")])));
        assert!(!same_origin(&headers(&[("sec-fetch-site", "same-site")])));
    }
}
