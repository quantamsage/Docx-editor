//! HTTP routes.
//!
//! * `GET /` — the editor page.
//! * `GET /static/{*path}` — embedded CSS, JavaScript and images.
//! * `/api/...` — JSON API used by the frontend. Unknown API routes return a
//!   JSON 404 so the frontend can always parse errors.

use axum::Json;
use axum::Router;
use axum::extract::Path;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde::Serialize;

use crate::assets::{self, Asset};
use crate::error::ApiError;
use crate::security;

/// Application name reported by the API.
pub const APP_NAME: &str = "docx-editor";

/// Builds the application router with all middleware.
pub fn router() -> Router {
    let api = Router::new().route("/health", get(health)).fallback(api_not_found);

    Router::new()
        .route("/", get(index))
        .route("/static/{*path}", get(static_asset))
        .nest("/api", api)
        .fallback(page_not_found)
        .layer(middleware::from_fn(security::local_only))
        .layer(middleware::from_fn(security::security_headers))
}

/// Response of `GET /api/health`.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Health {
    pub status: &'static str,
    pub app: &'static str,
    pub version: &'static str,
}

async fn health() -> Json<Health> {
    Json(Health { status: "ok", app: APP_NAME, version: env!("CARGO_PKG_VERSION") })
}

async fn index() -> Response {
    asset_response(&assets::INDEX)
}

async fn static_asset(Path(path): Path<String>) -> Response {
    match assets::find(&path) {
        Some(asset) => asset_response(asset),
        None => page_not_found().await.into_response(),
    }
}

fn asset_response(asset: &'static Asset) -> Response {
    let mut response = asset.body.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(asset.content_type));
    // Assets change with each build; always revalidate.
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"));
    response
}

async fn api_not_found() -> ApiError {
    ApiError::not_found("No such API endpoint.")
}

async fn page_not_found() -> (StatusCode, &'static str) {
    (StatusCode::NOT_FOUND, "Not found")
}
