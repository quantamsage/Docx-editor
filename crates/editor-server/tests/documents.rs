//! HTTP tests of the document endpoints.

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use docx_editor::app;
use docx_editor::documents::{AppState, MAX_OPEN_DOCUMENTS, MAX_UPLOAD_BYTES, UNTITLED};
use docx_engine::Document;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

const WORD_SAMPLE: &[u8] = include_bytes!("../../../tests/fixtures/word-sample.docx");

struct Response {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

impl Response {
    fn json(&self) -> Value {
        serde_json::from_slice(&self.body)
            .unwrap_or_else(|e| panic!("invalid JSON ({e}): {:?}", String::from_utf8_lossy(&self.body)))
    }

    fn header(&self, name: &str) -> &str {
        self.headers.get(name).and_then(|v| v.to_str().ok()).unwrap_or("")
    }
}

struct Client {
    router: Router,
}

impl Client {
    fn new() -> Self {
        Self { router: app::router_with_state(AppState::default()) }
    }

    async fn send(&self, method: Method, uri: &str, body: Vec<u8>) -> Response {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "localhost:8080")
            .header(header::ORIGIN, "http://localhost:8080")
            .header(header::CONTENT_TYPE, "application/octet-stream")
            .body(Body::from(body))
            .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes().to_vec();
        Response { status, headers, body }
    }

    async fn open(&self, name: &str, bytes: &[u8]) -> Response {
        self.send(Method::POST, &format!("/api/documents/open?name={name}"), bytes.to_vec()).await
    }
}

fn parts(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|i| {
            let mut file = archive.by_index(i).unwrap();
            let mut data = Vec::new();
            std::io::Read::read_to_end(&mut file, &mut data).unwrap();
            (file.name().unwrap().into_owned(), data)
        })
        .collect()
}

#[tokio::test]
async fn creates_fetches_downloads_and_closes_a_blank_document() {
    let client = Client::new();
    let created = client.send(Method::POST, "/api/documents/new", vec![]).await;
    assert_eq!(created.status, StatusCode::CREATED);
    let json = created.json();
    let id = json["id"].as_u64().unwrap();
    assert_eq!(json["name"], UNTITLED);
    assert_eq!(json["modified"], false);
    assert_eq!(json["model"]["blocks"].as_array().unwrap().len(), 1);
    assert_eq!(json["model"]["section"]["page_width"], 11906);

    let fetched = client.send(Method::GET, &format!("/api/documents/{id}"), vec![]).await;
    assert_eq!(fetched.status, StatusCode::OK);
    assert_eq!(fetched.json()["model"], json["model"]);

    let download = client.send(Method::GET, &format!("/api/documents/{id}/download"), vec![]).await;
    assert_eq!(download.status, StatusCode::OK);
    assert_eq!(
        download.header("content-type"),
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
    );
    assert_eq!(
        download.header("content-disposition"),
        "attachment; filename=\"Untitled document.docx\"; filename*=UTF-8''Untitled%20document.docx"
    );
    assert_eq!(download.header("cache-control"), "no-store");
    Document::open(&download.body).expect("downloaded file is a valid document");

    let closed = client.send(Method::DELETE, &format!("/api/documents/{id}"), vec![]).await;
    assert_eq!(closed.status, StatusCode::NO_CONTENT);
    for (method, path) in
        [(Method::GET, format!("/api/documents/{id}")), (Method::DELETE, format!("/api/documents/{id}"))]
    {
        let res = client.send(method, &path, vec![]).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND);
        assert_eq!(res.json()["error"]["code"], "not_found");
    }
}

#[tokio::test]
async fn opens_a_word_document_and_saves_it_unchanged() {
    let client = Client::new();
    let opened = client.open("Quarterly%20Report.docx", WORD_SAMPLE).await;
    assert_eq!(opened.status, StatusCode::CREATED, "{:?}", String::from_utf8_lossy(&opened.body));
    let json = opened.json();
    assert_eq!(json["name"], "Quarterly Report.docx");
    let stats = &json["model"]["stats"];
    assert_eq!(
        (stats["tables"].as_u64(), stats["images"].as_u64(), stats["hyperlinks"].as_u64()),
        (Some(1), Some(1), Some(1))
    );
    assert_eq!(json["model"]["blocks"][0]["style_id"], "Title");

    let id = json["id"].as_u64().unwrap();
    let download = client.send(Method::GET, &format!("/api/documents/{id}/download"), vec![]).await;
    assert_eq!(download.status, StatusCode::OK);
    assert_eq!(parts(&download.body), parts(WORD_SAMPLE), "every part is preserved");
}

#[tokio::test]
async fn save_as_renames_the_document() {
    let client = Client::new();
    let id = client.send(Method::POST, "/api/documents/new", vec![]).await.json()["id"].as_u64().unwrap();
    let download =
        client.send(Method::GET, &format!("/api/documents/{id}/download?name=Final%20plan.docm"), vec![]).await;
    assert_eq!(download.status, StatusCode::OK);
    assert_eq!(download.header("content-type"), "application/vnd.ms-word.document.macroEnabled.12");
    assert!(download.header("content-disposition").contains("filename=\"Final plan.docm\""));
    assert_eq!(
        client.send(Method::GET, &format!("/api/documents/{id}"), vec![]).await.json()["name"],
        "Final plan.docm"
    );

    let unsafe_name =
        client.send(Method::GET, &format!("/api/documents/{id}/download?name=..%2F..%2Fevil"), vec![]).await;
    assert!(unsafe_name.header("content-disposition").contains("filename=\"evil.docx\""));
}

#[tokio::test]
async fn rejects_invalid_files_with_readable_errors() {
    let client = Client::new();
    let mut ole = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    ole.resize(1024, 0);
    let mut not_word = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    not_word.start_file("hello.txt", zip::write::SimpleFileOptions::default()).unwrap();
    std::io::Write::write_all(&mut not_word, b"hi").unwrap();
    let not_word = not_word.finish().unwrap().into_inner();

    let cases: Vec<(Vec<u8>, StatusCode, &str, &str)> = vec![
        (b"definitely not a docx".to_vec(), StatusCode::UNPROCESSABLE_ENTITY, "invalid_document", "not a valid .docx"),
        (ole, StatusCode::UNPROCESSABLE_ENTITY, "unsupported_document", "password-protected"),
        (not_word, StatusCode::UNPROCESSABLE_ENTITY, "invalid_document", "not a Word document"),
        (vec![], StatusCode::BAD_REQUEST, "empty_file", "empty"),
    ];
    for (bytes, status, code, needle) in cases {
        let res = client.open("x.docx", &bytes).await;
        assert_eq!(res.status, status, "{code}");
        let json = res.json();
        assert_eq!(json["error"]["code"], code);
        assert!(json["error"]["message"].as_str().unwrap().contains(needle), "{json}");
    }
}

#[tokio::test]
async fn rejects_oversized_uploads() {
    let client = Client::new();
    let res = client.open("big.docx", &vec![b'P'; MAX_UPLOAD_BYTES + 1]).await;
    assert_eq!(res.status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(res.json()["error"]["code"], "document_too_large");
}

#[tokio::test]
async fn limits_the_number_of_open_documents() {
    let client = Client::new();
    for _ in 0..MAX_OPEN_DOCUMENTS {
        assert_eq!(client.send(Method::POST, "/api/documents/new", vec![]).await.status, StatusCode::CREATED);
    }
    let res = client.send(Method::POST, "/api/documents/new", vec![]).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.json()["error"]["code"], "too_many_documents");
}

#[tokio::test]
async fn documents_are_isolated_per_server_state() {
    let first = Client::new();
    let id = first.send(Method::POST, "/api/documents/new", vec![]).await.json()["id"].as_u64().unwrap();
    let second = Client::new();
    assert_eq!(second.send(Method::GET, &format!("/api/documents/{id}"), vec![]).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn document_endpoints_reject_cross_origin_requests() {
    let router = app::router();
    for (method, uri) in [
        (Method::POST, "/api/documents/new"),
        (Method::POST, "/api/documents/open"),
        (Method::DELETE, "/api/documents/1"),
    ] {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "localhost:8080")
            .header(header::ORIGIN, "https://evil.example")
            .body(Body::empty())
            .unwrap();
        assert_eq!(router.clone().oneshot(request).await.unwrap().status(), StatusCode::FORBIDDEN, "{uri}");
    }
}
