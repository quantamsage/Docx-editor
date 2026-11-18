//! HTTP-level tests of the router and of a real server on a loopback socket.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use docx_editor::app;
use http_body_util::BodyExt;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tower::ServiceExt;

struct TestResponse {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: String,
}

impl TestResponse {
    fn header(&self, name: &str) -> &str {
        self.headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
    }

    fn json(&self) -> serde_json::Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("invalid JSON ({e}): {}", self.body))
    }
}

async fn send(request: Request<Body>) -> TestResponse {
    let response = app::router().oneshot(request).await.expect("router is infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response.into_body().collect().await.expect("body").to_bytes();
    TestResponse { status, headers, body: String::from_utf8_lossy(&bytes).into_owned() }
}

fn get(path: &str) -> Request<Body> {
    Request::get(path).header(header::HOST, "localhost:8080").body(Body::empty()).unwrap()
}

#[tokio::test]
async fn serves_the_editor_page() {
    let res = send(get("/")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert_eq!(res.header("content-type"), "text/html; charset=utf-8");
    for needle in [
        "<title>Untitled document – DOCX Editor</title>",
        r#"<script type="module" src="/static/js/app.js"></script>"#,
        r#"href="/static/css/app.css""#,
        r#"class="toolbar toolbar-main""#,
        r#"class="toolbar toolbar-format""#,
        r#"class="page""#,
        r#"id="status-connection""#,
    ] {
        assert!(res.body.contains(needle), "page is missing {needle}");
    }
}

#[tokio::test]
async fn serves_static_assets_with_types() {
    for (path, content_type, needle) in [
        ("/static/css/app.css", "text/css; charset=utf-8", ".workspace"),
        ("/static/js/app.js", "text/javascript; charset=utf-8", "createConnectionMonitor"),
        ("/static/js/api.js", "text/javascript; charset=utf-8", "export async function getHealth"),
        ("/static/js/status.js", "text/javascript; charset=utf-8", "export function countWords"),
        ("/static/favicon.svg", "image/svg+xml", "<svg"),
    ] {
        let res = send(get(path)).await;
        assert_eq!(res.status, StatusCode::OK, "{path}");
        assert_eq!(res.header("content-type"), content_type, "{path}");
        assert_eq!(res.header("cache-control"), "no-cache", "{path}");
        assert!(res.body.contains(needle), "{path} body");
    }
}

#[tokio::test]
async fn every_script_and_stylesheet_referenced_by_the_page_is_served() {
    let page = send(get("/")).await.body;
    let mut referenced: Vec<String> = Vec::new();
    for attr in ["src=\"", "href=\""] {
        for part in page.split(attr).skip(1) {
            let url = part.split('"').next().unwrap();
            if url.starts_with("/static/") {
                referenced.push(url.to_string());
            }
        }
    }
    // app.js imports its sibling modules by relative path.
    let app_js = send(get("/static/js/app.js")).await.body;
    for part in app_js.split("from \"./").skip(1) {
        referenced.push(format!("/static/js/{}", part.split('"').next().unwrap()));
    }
    assert!(referenced.len() >= 5, "found {referenced:?}");
    for url in referenced {
        assert_eq!(send(get(&url)).await.status, StatusCode::OK, "{url}");
    }
}

#[tokio::test]
async fn unknown_paths_are_not_found() {
    for path in ["/missing", "/static/js/missing.js", "/static/../Cargo.toml", "/static/%2e%2e/Cargo.toml", "/static/"]
    {
        assert_eq!(send(get(path)).await.status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn health_endpoint_reports_version() {
    let res = send(get("/api/health")).await;
    assert_eq!(res.status, StatusCode::OK);
    assert!(res.header("content-type").starts_with("application/json"));
    assert_eq!(
        res.json(),
        serde_json::json!({"status": "ok", "app": "docx-editor", "version": env!("CARGO_PKG_VERSION")})
    );
}

#[tokio::test]
async fn unknown_api_routes_return_json_errors() {
    let res = send(get("/api/nope")).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
    assert_eq!(res.json(), serde_json::json!({"error": {"code": "not_found", "message": "No such API endpoint."}}));

    let res = send(Request::post("/api/health").header(header::HOST, "localhost").body(Body::empty()).unwrap()).await;
    assert_eq!(res.status, StatusCode::METHOD_NOT_ALLOWED);
}

#[tokio::test]
async fn sets_security_headers() {
    for path in ["/", "/api/health", "/static/js/app.js", "/missing"] {
        let res = send(get(path)).await;
        let csp = res.header("content-security-policy");
        assert!(csp.contains("default-src 'self'") && csp.contains("script-src 'self'"), "{path}: {csp}");
        assert!(csp.contains("frame-ancestors 'none'"), "{path}");
        assert_eq!(res.header("x-content-type-options"), "nosniff", "{path}");
        assert_eq!(res.header("x-frame-options"), "DENY", "{path}");
        assert_eq!(res.header("referrer-policy"), "no-referrer", "{path}");
    }
}

#[tokio::test]
async fn page_has_no_inline_scripts_or_styles() {
    // The CSP forbids them, so they would silently not work.
    let page = send(get("/")).await.body;
    assert!(!page.contains("<style"), "inline <style>");
    assert!(!page.contains(" style="), "inline style attribute");
    for script in page.split("<script").skip(1) {
        assert!(script.trim_start().starts_with("type=\"module\" src="), "inline script: {script}");
    }
    assert!(!page.contains("http://") && !page.contains("https://"), "page references a remote resource");
}

#[tokio::test]
async fn rejects_non_loopback_host_headers() {
    for host in ["evil.example", "evil.example:8080", "localhost.evil.example", "192.168.1.5:8080"] {
        let req = Request::get("/api/health").header(header::HOST, host).body(Body::empty()).unwrap();
        let res = send(req).await;
        assert_eq!(res.status, StatusCode::FORBIDDEN, "{host}");
        assert_eq!(res.json()["error"]["code"], "forbidden", "{host}");
    }
    let no_host = send(Request::get("/").body(Body::empty()).unwrap()).await;
    assert_eq!(no_host.status, StatusCode::FORBIDDEN);

    for host in ["localhost", "127.0.0.1:8080", "[::1]:9000"] {
        let req = Request::get("/api/health").header(header::HOST, host).body(Body::empty()).unwrap();
        assert_eq!(send(req).await.status, StatusCode::OK, "{host}");
    }
}

#[tokio::test]
async fn rejects_cross_origin_state_changes() {
    let cross = Request::builder()
        .method(Method::POST)
        .uri("/api/health")
        .header(header::HOST, "localhost:8080")
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(cross).await.status, StatusCode::FORBIDDEN);

    let same = Request::builder()
        .method(Method::POST)
        .uri("/api/health")
        .header(header::HOST, "localhost:8080")
        .header(header::ORIGIN, "http://localhost:8080")
        .body(Body::empty())
        .unwrap();
    // Allowed through the origin check; the route itself only supports GET.
    assert_eq!(send(same).await.status, StatusCode::METHOD_NOT_ALLOWED);

    // Cross-origin GETs are harmless to the server state and pass the check.
    let cross_get = Request::get("/api/health")
        .header(header::HOST, "localhost:8080")
        .header(header::ORIGIN, "https://evil.example")
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(cross_get).await.status, StatusCode::OK);
}

/// Starts the real server on an OS-assigned loopback port and talks raw
/// HTTP/1.1 to it, then shuts it down gracefully.
#[tokio::test]
async fn real_server_serves_over_tcp_and_shuts_down() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (stop_tx, stop_rx) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(docx_editor::serve(listener, async {
        stop_rx.await.ok();
    }));

    let mut stream = TcpStream::connect(addr).await.unwrap();
    let request = format!("GET /api/health HTTP/1.1\r\nHost: localhost:{}\r\nConnection: close\r\n\r\n", addr.port());
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut raw = String::new();
    stream.read_to_string(&mut raw).await.unwrap();

    assert!(raw.starts_with("HTTP/1.1 200 OK"), "{raw}");
    assert!(raw.contains(r#""status":"ok""#), "{raw}");

    stop_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), server)
        .await
        .expect("server shut down in time")
        .expect("server task")
        .expect("server result");
}
