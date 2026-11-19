//! Local web-based DOCX editor: HTTP server and embedded frontend.
//!
//! The server is a thin layer: it serves the frontend and exposes a JSON API.
//! Document processing lives in the `docx-engine` crate, which has no
//! knowledge of HTTP.

pub mod app;
pub mod assets;
pub mod config;
pub mod documents;
pub mod error;
pub mod security;

use std::future::Future;

use tokio::net::TcpListener;

/// Serves the application on `listener` until `shutdown` completes, then
/// finishes in-flight requests and returns.
pub async fn serve(listener: TcpListener, shutdown: impl Future<Output = ()> + Send + 'static) -> std::io::Result<()> {
    axum::serve(listener, app::router()).with_graceful_shutdown(shutdown).await
}
