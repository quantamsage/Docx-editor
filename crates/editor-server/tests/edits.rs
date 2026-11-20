//! HTTP tests of editing (`POST /api/documents/{id}/edits`) and image
//! display (`GET /api/documents/{id}/media/{part}`).

use axum::Router;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode, header};
use docx_editor::app;
use docx_editor::documents::AppState;
use docx_engine::Document;
use http_body_util::BodyExt;
use serde_json::{Value, json};
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
}

struct Client {
    router: Router,
}

impl Client {
    fn new() -> Self {
        Self { router: app::router_with_state(AppState::default()) }
    }

    async fn send(&self, method: Method, uri: &str, content_type: &str, body: Vec<u8>) -> Response {
        let request = Request::builder()
            .method(method)
            .uri(uri)
            .header(header::HOST, "localhost:8080")
            .header(header::ORIGIN, "http://localhost:8080")
            .header(header::CONTENT_TYPE, content_type)
            .body(Body::from(body))
            .unwrap();
        let response = self.router.clone().oneshot(request).await.unwrap();
        let status = response.status();
        let headers = response.headers().clone();
        let body = response.into_body().collect().await.unwrap().to_bytes().to_vec();
        Response { status, headers, body }
    }

    async fn new_document(&self) -> u64 {
        let res = self.send(Method::POST, "/api/documents/new", "application/json", vec![]).await;
        res.json()["id"].as_u64().unwrap()
    }

    async fn edit(&self, id: u64, body: Value) -> Response {
        self.send(
            Method::POST,
            &format!("/api/documents/{id}/edits"),
            "application/json",
            body.to_string().into_bytes(),
        )
        .await
    }

    async fn download(&self, id: u64) -> Vec<u8> {
        let res = self.send(Method::GET, &format!("/api/documents/{id}/download"), "application/json", vec![]).await;
        assert_eq!(res.status, StatusCode::OK);
        res.body
    }
}

fn texts(model: &Value) -> Vec<String> {
    model["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|b| b["type"] == "paragraph")
        .map(|p| {
            p["inlines"]
                .as_array()
                .unwrap()
                .iter()
                .flat_map(|i| i["content"].as_array().cloned().unwrap_or_default())
                .filter_map(|c| c["text"].as_str().map(str::to_owned))
                .collect::<String>()
        })
        .collect()
}

#[tokio::test]
async fn edits_apply_bump_the_revision_and_are_saved() {
    let client = Client::new();
    let id = client.new_document().await;

    let res = client
        .edit(
            id,
            json!({"base_revision": 0, "edits": [{"op": "splice", "first": 0, "last": 0, "content": [
                {"type": "text", "text": "Hello"},
                {"type": "break"},
                {"type": "text", "text": "world"}
            ]}]}),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", String::from_utf8_lossy(&res.body));
    let body = res.json();
    assert_eq!(body["revision"], 1);
    assert_eq!(body["modified"], true);
    assert_eq!(texts(&body["model"]), ["Hello", "world"]);

    // Keep the first run and type after it, formatted like it.
    let res = client
        .edit(
            id,
            json!({"base_revision": 1, "edits": [{"op": "splice", "first": 0, "last": 0, "content": [
                {"type": "keep", "block": 0, "inline": 0},
                {"type": "text", "text": ", there", "like": {"block": 0, "inline": 0}}
            ]}]}),
        )
        .await;
    assert_eq!(res.json()["revision"], 2);

    let saved = Document::open(&client.download(id).await).unwrap().model();
    let saved = serde_json::to_value(&saved).unwrap();
    assert_eq!(texts(&saved), ["Hello, there", "world"]);

    // Fetching reports the same revision.
    let fetched = client.send(Method::GET, &format!("/api/documents/{id}"), "application/json", vec![]).await;
    assert_eq!(fetched.json()["revision"], 2);
}

#[tokio::test]
async fn stale_and_invalid_edits_change_nothing() {
    let client = Client::new();
    let id = client.new_document().await;
    let valid = json!({"op": "splice", "first": 0, "last": 0, "content": [{"type": "text", "text": "x"}]});

    let res = client.edit(id, json!({"base_revision": 5, "edits": [valid]})).await;
    assert_eq!(res.status, StatusCode::CONFLICT);
    assert_eq!(res.json()["error"]["code"], "stale_revision");

    // An invalid edit in a batch rolls back the whole batch.
    let res = client.edit(id, json!({"base_revision": 0, "edits": [valid, {"op": "delete_block", "block": 9}]})).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    assert_eq!(res.json()["error"]["code"], "invalid_edit");
    assert!(res.json()["error"]["message"].as_str().unwrap().contains("edit 2 of 2"));

    for body in [
        json!({"edits": []}),
        json!({"base_revision": 0, "edits": [{"op": "rm_rf"}]}),
        json!({"base_revision": 0, "edits": [], "extra": 1}),
        json!({"base_revision": 0, "edits": [{"op": "splice", "first": 0, "last": 0, "content": [{"type": "text", "text": "\u{1}"}]}]}),
    ] {
        let res = client.edit(id, body.clone()).await;
        assert!(
            res.status == StatusCode::BAD_REQUEST || res.status == StatusCode::UNPROCESSABLE_ENTITY,
            "{body}: {}",
            res.status
        );
        assert!(res.json()["error"]["code"].is_string());
    }

    let fetched = client.send(Method::GET, &format!("/api/documents/{id}"), "application/json", vec![]).await.json();
    assert_eq!(fetched["revision"], 0);
    assert_eq!(fetched["modified"], false);

    // An empty batch is a no-op that returns the current state.
    let res = client.edit(id, json!({"base_revision": 0, "edits": []})).await;
    assert_eq!((res.status, res.json()["revision"].as_u64()), (StatusCode::OK, Some(0)));

    // Unknown documents.
    assert_eq!(client.edit(999, json!({"base_revision": 0, "edits": []})).await.status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn edits_reject_cross_origin_requests_and_bad_content_types() {
    let client = Client::new();
    let id = client.new_document().await;
    let body = json!({"base_revision": 0, "edits": []}).to_string();
    let request = Request::builder()
        .method(Method::POST)
        .uri(format!("/api/documents/{id}/edits"))
        .header(header::HOST, "localhost:8080")
        .header(header::ORIGIN, "https://evil.example")
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.clone()))
        .unwrap();
    let res = client.router.clone().oneshot(request).await.unwrap();
    assert_eq!(res.status(), StatusCode::FORBIDDEN);

    let res = client.send(Method::POST, &format!("/api/documents/{id}/edits"), "text/plain", body.into_bytes()).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST, "only JSON bodies are accepted");
}

#[tokio::test]
async fn editing_a_word_document_keeps_other_parts() {
    let client = Client::new();
    let res = client
        .send(Method::POST, "/api/documents/open?name=sample.docx", "application/octet-stream", WORD_SAMPLE.to_vec())
        .await;
    let json = res.json();
    let id = json["id"].as_u64().unwrap();
    let blocks = json["model"]["blocks"].as_array().unwrap();
    let index =
        blocks.iter().position(|b| b["type"] == "paragraph" && !b["inlines"].as_array().unwrap().is_empty()).unwrap();
    let inlines = blocks[index]["inlines"].as_array().unwrap().len();
    let mut content: Vec<Value> = (0..inlines).map(|i| json!({"type": "keep", "block": index, "inline": i})).collect();
    content.push(json!({"type": "text", "text": " — edited in the browser"}));
    let res = client
        .edit(
            id,
            json!({"base_revision": 0, "edits": [{"op": "splice", "first": index, "last": index, "content": content}]}),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK);

    let saved = client.download(id).await;
    let reopened = serde_json::to_value(Document::open(&saved).unwrap().model()).unwrap();
    assert!(texts(&reopened)[..].iter().any(|t| t.ends_with(" — edited in the browser")));

    let parts = |bytes: &[u8]| {
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes.to_vec())).unwrap();
        (0..archive.len())
            .map(|i| {
                let mut f = archive.by_index(i).unwrap();
                let mut data = Vec::new();
                std::io::Read::read_to_end(&mut f, &mut data).unwrap();
                (f.name().unwrap().into_owned(), data)
            })
            .collect::<Vec<_>>()
    };
    let before = parts(WORD_SAMPLE);
    for (name, data) in parts(&saved) {
        if name != "word/document.xml" {
            assert_eq!(Some(&data), before.iter().find(|(n, _)| *n == name).map(|(_, d)| d), "{name}");
        }
    }
}

fn find_image_part(model: &Value) -> Option<String> {
    fn walk(v: &Value) -> Option<String> {
        match v {
            Value::Object(map) => {
                if map.get("type").and_then(Value::as_str) == Some("image") {
                    return map.get("part").and_then(Value::as_str).map(str::to_owned);
                }
                map.values().find_map(walk)
            }
            Value::Array(items) => items.iter().find_map(walk),
            _ => None,
        }
    }
    walk(model)
}

#[tokio::test]
async fn images_referenced_by_the_document_are_served() {
    let client = Client::new();
    let res = client
        .send(Method::POST, "/api/documents/open?name=sample.docx", "application/octet-stream", WORD_SAMPLE.to_vec())
        .await;
    let json = res.json();
    let id = json["id"].as_u64().unwrap();
    let part = find_image_part(&json["model"]).expect("the Word sample has a picture");

    let res = client.send(Method::GET, &format!("/api/documents/{id}/media/{part}"), "application/json", vec![]).await;
    assert_eq!(res.status, StatusCode::OK, "{part}");
    assert!(res.headers[header::CONTENT_TYPE].to_str().unwrap().starts_with("image/"));
    assert_eq!(res.headers["x-content-type-options"], "nosniff");
    assert!(!res.body.is_empty());

    // Other parts, unknown parts and traversal are not served.
    for path in [
        "word/document.xml",
        "word/styles.xml",
        "word/media/missing.png",
        "../../Cargo.toml",
        "%2e%2e/word/document.xml",
    ] {
        let res =
            client.send(Method::GET, &format!("/api/documents/{id}/media/{path}"), "application/json", vec![]).await;
        assert_eq!(res.status, StatusCode::NOT_FOUND, "{path}");
    }
    let res = client.send(Method::GET, &format!("/api/documents/999/media/{part}"), "application/json", vec![]).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}
