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

#[tokio::test]
async fn formatting_edits_apply_through_the_api() {
    let client = Client::new();
    let id = client.new_document().await;
    let res = client
        .edit(
            id,
            json!({"base_revision": 0, "edits": [
                {"op": "splice", "first": 0, "last": 0, "content": [{"type": "text", "text": "Bold title", "format": {"bold": true, "size": 32}}]},
                {"op": "format_paragraphs", "first": 0, "last": 0, "format": {"style_id": "Heading1", "alignment": "center", "list": {"kind": "number"}}}
            ]}),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", String::from_utf8_lossy(&res.body));
    let model = &res.json()["model"];
    let p = &model["blocks"][0];
    assert_eq!(p["style_id"], "Heading1");
    assert_eq!(p["alignment"], "center");
    assert_eq!(p["inlines"][0]["props"]["bold"], true);
    assert_eq!(p["inlines"][0]["props"]["size"], 32);
    assert_eq!(model["lists"][0]["levels"][0]["format"], "decimal");
    assert_eq!(p["numbering"]["num_id"], model["lists"][0]["num_id"]);

    // Invalid formats are refused and change nothing.
    let res = client
        .edit(id, json!({"base_revision": 1, "edits": [{"op": "format_paragraphs", "first": 0, "last": 0, "format": {"spacing": {"line": 1}}}]}))
        .await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    let saved = Document::open(&client.download(id).await).unwrap().model();
    assert_eq!(saved.lists.len(), 1);
}

/// A 1×1 PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52, 0x00, 0x00, 0x00,
    0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4, 0x89, 0x00, 0x00, 0x00, 0x0D, 0x49,
    0x44, 0x41, 0x54, 0x78, 0x9C, 0x63, 0xF8, 0xCF, 0xC0, 0xF0, 0x1F, 0x00, 0x05, 0x00, 0x01, 0xFF, 0x89, 0x99, 0x3D,
    0x1D, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4E, 0x44, 0xAE, 0x42, 0x60, 0x82,
];

#[tokio::test]
async fn pictures_are_uploaded_then_inserted_and_served() {
    let client = Client::new();
    let id = client.new_document().await;
    let res = client.send(Method::POST, &format!("/api/documents/{id}/images"), "image/png", PNG.to_vec()).await;
    assert_eq!(res.status, StatusCode::CREATED, "{:?}", String::from_utf8_lossy(&res.body));
    let info = res.json();
    assert_eq!((info["format"].as_str(), info["width_px"].as_u64()), (Some("png"), Some(1)));
    let image = info["id"].as_str().unwrap().to_string();

    let res = client
        .edit(
            id,
            json!({"base_revision": 0, "edits": [{"op": "splice", "first": 0, "last": 0, "content": [
                {"type": "image", "image": image, "width_emu": 95250, "height_emu": 95250, "description": "Dot"}
            ]}]}),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", String::from_utf8_lossy(&res.body));
    let part = find_image_part(&res.json()["model"]).expect("the picture is in the model");
    let res = client.send(Method::GET, &format!("/api/documents/{id}/media/{part}"), "application/json", vec![]).await;
    assert_eq!((res.status, res.body.as_slice()), (StatusCode::OK, PNG));
    assert_eq!(Document::open(&client.download(id).await).unwrap().model().stats.images, 1);

    for (body, status) in [
        (b"not a picture".to_vec(), StatusCode::UNPROCESSABLE_ENTITY),
        (Vec::new(), StatusCode::BAD_REQUEST),
        (vec![0u8; MAX_IMAGE + 1], StatusCode::PAYLOAD_TOO_LARGE),
    ] {
        let res = client.send(Method::POST, &format!("/api/documents/{id}/images"), "image/png", body).await;
        assert_eq!(res.status, status);
        assert!(res.json()["error"]["message"].is_string());
    }
    let res = client.send(Method::POST, "/api/documents/999/images", "image/png", PNG.to_vec()).await;
    assert_eq!(res.status, StatusCode::NOT_FOUND);
}

const MAX_IMAGE: usize = docx_editor::documents::MAX_IMAGE_BYTES;

#[tokio::test]
async fn tables_headers_and_page_setup_through_the_api() {
    let client = Client::new();
    let id = client.new_document().await;
    let res = client
        .edit(
            id,
            json!({"base_revision": 0, "edits": [
                {"op": "insert_table", "at": 1, "rows": 2, "columns": 2},
                {"op": "splice", "container": {"cell": {"table": 1, "row": 0, "cell": 1}}, "first": 0, "last": 0,
                 "content": [{"type": "text", "text": "Cell text"}]},
                {"op": "insert_page_numbers", "kind": "footer", "alignment": "right"},
                {"op": "format_section", "format": {"landscape": true}}
            ]}),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", String::from_utf8_lossy(&res.body));
    let model = &res.json()["model"];
    assert_eq!(model["blocks"][1]["rows"][0]["cells"][1]["blocks"][0]["inlines"][0]["content"][0]["text"], "Cell text");
    assert_eq!(model["section"]["landscape"], true);
    let footer = &model["footer"];
    assert_eq!(footer["part"], "word/footer1.xml");
    assert_eq!(footer["blocks"][0]["inlines"][0]["field"], "PAGE");

    // The footer is edited through its part.
    let res = client
        .edit(
            id,
            json!({"base_revision": 1, "edits": [{"op": "splice", "container": {"part": "word/footer1.xml"}, "first": 0, "last": 0,
                "content": [{"type": "text", "text": "Page "}, {"type": "keep", "block": 0, "inline": 0}]}]}),
        )
        .await;
    assert_eq!(res.status, StatusCode::OK, "{:?}", String::from_utf8_lossy(&res.body));
    let saved = Document::open(&client.download(id).await).unwrap().model();
    let Some(footer) = saved.footer else { panic!("footer saved") };
    let docx_engine::model::Block::Paragraph(p) = &footer.blocks[0] else { panic!() };
    assert_eq!(p.plain_text(), "Page 1");
}

async fn step(client: &Client, id: u64, op: &str, base: u64) -> Response {
    let body = json!({"base_revision": base}).to_string().into_bytes();
    client.send(Method::POST, &format!("/api/documents/{id}/{op}"), "application/json", body).await
}

#[tokio::test]
async fn undo_and_redo_step_through_edit_batches() {
    let client = Client::new();
    let id = client.new_document().await;
    let typed = |text: &str, base: u64, group: Option<&str>| {
        let mut body = json!({"base_revision": base, "edits": [{"op": "splice", "first": 0, "last": 0, "content": [{"type": "text", "text": text}]}]});
        if let Some(group) = group {
            body["group"] = json!(group);
        }
        body
    };

    let fresh = client.send(Method::GET, &format!("/api/documents/{id}"), "application/json", vec![]).await.json();
    assert_eq!((fresh["can_undo"].as_bool(), fresh["can_redo"].as_bool()), (Some(false), Some(false)));
    let res = step(&client, id, "undo", 0).await;
    assert_eq!((res.status, res.json()["error"]["code"].as_str()), (StatusCode::CONFLICT, Some("nothing_to_undo")));

    // Two batches of typing in one group, then a separate edit.
    client.edit(id, typed("H", 0, Some("typing-1"))).await;
    client.edit(id, typed("Hi", 1, Some("typing-1"))).await;
    let res = client.edit(id, typed("Hi there", 2, None)).await.json();
    assert_eq!((res["revision"].as_u64(), res["can_undo"].as_bool()), (Some(3), Some(true)));

    let res = step(&client, id, "undo", 3).await;
    assert_eq!(res.status, StatusCode::OK);
    let body = res.json();
    assert_eq!(texts(&body["model"]), ["Hi"]);
    assert_eq!((body["revision"].as_u64(), body["can_redo"].as_bool()), (Some(4), Some(true)));
    let body = step(&client, id, "undo", 4).await.json();
    assert_eq!(texts(&body["model"]), [""], "the typing group is one step");
    assert_eq!((body["can_undo"].as_bool(), body["modified"].as_bool()), (Some(false), Some(false)));

    let body = step(&client, id, "redo", 5).await.json();
    assert_eq!(texts(&body["model"]), ["Hi"]);
    assert_eq!(step(&client, id, "redo", 5).await.status, StatusCode::CONFLICT, "stale revision");
    let body = step(&client, id, "redo", 6).await.json();
    assert_eq!(texts(&body["model"]), ["Hi there"]);
    assert_eq!(body["can_redo"], false);
    let saved = Document::open(&client.download(id).await).unwrap().model();
    assert_eq!(texts(&serde_json::to_value(&saved).unwrap()), ["Hi there"]);

    // A failed batch leaves no undo step.
    let res = client.edit(id, json!({"base_revision": 7, "edits": [{"op": "delete_block", "block": 9}]})).await;
    assert_eq!(res.status, StatusCode::BAD_REQUEST);
    let body = step(&client, id, "undo", 7).await.json();
    assert_eq!(texts(&body["model"]), ["Hi"]);
    for bad in [json!({}), json!({"base_revision": 8, "extra": true})] {
        let res = client
            .send(Method::POST, &format!("/api/documents/{id}/undo"), "application/json", bad.to_string().into_bytes())
            .await;
        assert_eq!(res.status, StatusCode::BAD_REQUEST);
    }
}
