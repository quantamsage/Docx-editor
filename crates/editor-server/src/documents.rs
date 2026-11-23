//! Open documents and the `/api/documents` endpoints.
//!
//! * `POST /api/documents/new` — create a blank document.
//! * `POST /api/documents/open?name=<file name>` — open the `.docx` bytes in
//!   the request body (the browser reads the file the user picked).
//! * `GET /api/documents/{id}` — the document's model.
//! * `POST /api/documents/{id}/edits` — apply edits atomically (JSON
//!   `{"base_revision": n, "edits": [...]}`) → the updated document.
//! * `POST /api/documents/{id}/images` — upload a picture (PNG, JPEG, GIF or
//!   BMP bytes) to insert with a later edit → `{id, format, width_px,
//!   height_px}`.
//! * `POST /api/documents/{id}/undo`, `POST /api/documents/{id}/redo` —
//!   undo or redo the last edit batch (JSON `{"base_revision": n}`).
//! * `GET /api/documents/{id}/media/{part}` — an image the document shows.
//! * `GET /api/documents/{id}/download?name=<file name>` — the saved `.docx`.
//! * `DELETE /api/documents/{id}` — close the document.
//!
//! Documents live only in memory and are only ever read from or written to
//! the user's disk by the browser, through files the user picks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::Json;
use axum::Router;
use axum::body::Bytes;
use axum::extract::rejection::BytesRejection;
use axum::extract::{DefaultBodyLimit, Path, Query, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use docx_engine::history::History;
use docx_engine::{Document, DocumentModel, DocxError, Edit};
use serde::{Deserialize, Serialize};

use crate::error::ApiError;

/// Largest `.docx` file accepted for opening.
pub const MAX_UPLOAD_BYTES: usize = 100 << 20;
/// Most documents open at once (each holds its parts in memory).
pub const MAX_OPEN_DOCUMENTS: usize = 32;
/// Name given to new documents.
pub const UNTITLED: &str = "Untitled document.docx";
/// Largest edit request (pasting a long text is one request).
pub const MAX_EDIT_BYTES: usize = 8 << 20;
/// Most edits in one request.
pub const MAX_EDITS: usize = 1000;
/// Largest picture accepted for insertion.
pub const MAX_IMAGE_BYTES: usize = 25 << 20;

const DOCX_MIME: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.document";
const DOCM_MIME: &str = "application/vnd.ms-word.document.macroEnabled.12";
const DOTX_MIME: &str = "application/vnd.openxmlformats-officedocument.wordprocessingml.template";
const DOTM_MIME: &str = "application/vnd.ms-word.template.macroEnabled.12";

/// An open document and the file name it is known by.
#[derive(Debug)]
pub struct OpenDocument {
    pub name: String,
    pub document: Document,
    /// Incremented by every applied edit batch. Edits address paragraphs
    /// and inlines by position, so a client must say which revision its
    /// positions refer to. Undo and redo also count as revisions.
    pub revision: u64,
    pub history: History,
}

impl OpenDocument {
    pub fn new(name: String, document: Document) -> Self {
        Self { name, document, revision: 0, history: History::new() }
    }
}

/// All open documents.
#[derive(Debug, Default)]
pub struct DocumentStore {
    next_id: u64,
    documents: HashMap<u64, Arc<Mutex<OpenDocument>>>,
}

impl DocumentStore {
    fn insert(&mut self, doc: OpenDocument) -> Result<u64, ApiError> {
        if self.documents.len() >= MAX_OPEN_DOCUMENTS {
            return Err(ApiError::new(
                StatusCode::CONFLICT,
                "too_many_documents",
                format!("At most {MAX_OPEN_DOCUMENTS} documents can be open at once. Close one and try again."),
            ));
        }
        self.next_id += 1;
        self.documents.insert(self.next_id, Arc::new(Mutex::new(doc)));
        Ok(self.next_id)
    }

    fn get(&self, id: u64) -> Result<Arc<Mutex<OpenDocument>>, ApiError> {
        self.documents.get(&id).cloned().ok_or_else(document_not_found)
    }
}

/// Shared server state.
#[derive(Debug, Clone, Default)]
pub struct AppState {
    documents: Arc<Mutex<DocumentStore>>,
}

impl AppState {
    fn store(&self) -> std::sync::MutexGuard<'_, DocumentStore> {
        // A panic while holding the lock cannot leave the map inconsistent
        // (every operation is a single insert/remove), so recover from poisoning.
        self.documents.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

/// The `/api/documents` routes.
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/new", post(create))
        .route("/open", post(open).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)))
        .route("/{id}", get(fetch).delete(close))
        .route("/{id}/download", get(download))
        .route("/{id}/edits", post(edit).layer(DefaultBodyLimit::max(MAX_EDIT_BYTES)))
        .route("/{id}/undo", post(undo))
        .route("/{id}/redo", post(redo))
        .route("/{id}/images", post(upload_image).layer(DefaultBodyLimit::max(MAX_IMAGE_BYTES)))
        .route("/{id}/media/{*part}", get(media))
}

/// A document as returned to the frontend.
#[derive(Debug, Serialize)]
pub struct DocumentResponse {
    pub id: u64,
    pub name: String,
    pub modified: bool,
    pub revision: u64,
    pub can_undo: bool,
    pub can_redo: bool,
    pub model: DocumentModel,
}

fn respond(id: u64, doc: &OpenDocument) -> DocumentResponse {
    DocumentResponse {
        id,
        name: doc.name.clone(),
        modified: doc.document.is_modified(),
        revision: doc.revision,
        can_undo: doc.history.can_undo(),
        can_redo: doc.history.can_redo(),
        model: doc.document.model(),
    }
}

#[derive(Debug, Deserialize)]
pub struct NameQuery {
    name: Option<String>,
}

async fn create(State(state): State<AppState>) -> Result<(StatusCode, Json<DocumentResponse>), ApiError> {
    let doc = OpenDocument::new(UNTITLED.to_string(), Document::new_blank());
    let response = respond(0, &doc);
    let id = state.store().insert(doc)?;
    Ok((StatusCode::CREATED, Json(DocumentResponse { id, ..response })))
}

async fn open(
    State(state): State<AppState>,
    Query(query): Query<NameQuery>,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<DocumentResponse>), ApiError> {
    let bytes = body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "document_too_large",
                format!("The file is larger than the {} MiB limit.", MAX_UPLOAD_BYTES >> 20),
            )
        } else {
            ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "The file could not be received.")
        }
    })?;
    if bytes.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "empty_file", "The file is empty."));
    }
    let name = sanitize_file_name(query.name.as_deref().unwrap_or(""));

    // Parsing can take a while for large documents; keep it off the async runtime.
    let (document, model) = tokio::task::spawn_blocking(move || {
        let document = Document::open(&bytes)?;
        let model = document.model();
        Ok::<_, DocxError>((document, model))
    })
    .await
    .map_err(|_| internal_error())?
    .map_err(docx_error)?;

    tracing::info!(name = %name, blocks = model.blocks.len(), "document opened");
    let id = state.store().insert(OpenDocument::new(name.clone(), document))?;
    let response = DocumentResponse { id, name, modified: false, revision: 0, can_undo: false, can_redo: false, model };
    Ok((StatusCode::CREATED, Json(response)))
}

async fn fetch(State(state): State<AppState>, Path(id): Path<u64>) -> Result<Json<DocumentResponse>, ApiError> {
    let entry = state.store().get(id)?;
    let response = tokio::task::spawn_blocking(move || {
        let doc = entry.lock().unwrap_or_else(|p| p.into_inner());
        respond(id, &doc)
    })
    .await
    .map_err(|_| internal_error())?;
    Ok(Json(response))
}

async fn download(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    Query(query): Query<NameQuery>,
) -> Result<Response, ApiError> {
    let entry = state.store().get(id)?;
    let requested = query.name.map(|n| sanitize_file_name(&n));
    let (name, bytes) = tokio::task::spawn_blocking(move || {
        let mut doc = entry.lock().unwrap_or_else(|p| p.into_inner());
        let bytes = doc.document.save()?;
        // "Save as" renames the open document.
        if let Some(name) = requested {
            doc.name = name;
        }
        Ok::<_, DocxError>((doc.name.clone(), bytes))
    })
    .await
    .map_err(|_| internal_error())?
    .map_err(docx_error)?;

    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime_for(&name)));
    if let Ok(value) = HeaderValue::from_str(&content_disposition(&name)) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

/// Body of `POST /api/documents/{id}/edits`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EditRequest {
    /// The revision the edits' positions refer to.
    base_revision: u64,
    edits: Vec<Edit>,
    /// Consecutive batches with the same group (e.g. typing in one
    /// paragraph) are undone together.
    #[serde(default)]
    group: Option<String>,
}

/// Body of `POST /api/documents/{id}/undo` and `/redo`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryRequest {
    base_revision: u64,
}

async fn edit(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    body: Result<Json<EditRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<DocumentResponse>, ApiError> {
    let Json(request) = body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "edit_too_large", "The change is too large to apply at once.")
        } else {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                "bad_request",
                format!("Invalid edit request: {}", rejection.body_text()),
            )
        }
    })?;
    if request.edits.len() > MAX_EDITS {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "Too many edits in one request."));
    }
    let entry = state.store().get(id)?;
    let response = tokio::task::spawn_blocking(move || {
        let mut doc = entry.lock().unwrap_or_else(|p| p.into_inner());
        if doc.revision != request.base_revision {
            return Err(stale_revision());
        }
        if !request.edits.is_empty() {
            let doc = &mut *doc;
            let recorded = doc.history.record(&doc.document, request.group.as_deref());
            if let Err(err) = doc.document.apply(&request.edits) {
                if recorded {
                    doc.history.forget_last();
                }
                return Err(docx_error(err));
            }
            doc.revision += 1;
        }
        Ok(respond(id, &doc))
    })
    .await
    .map_err(|_| internal_error())??;
    Ok(Json(response))
}

async fn upload_image(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    body: Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<docx_engine::media::StagedImageInfo>), ApiError> {
    let bytes = body.map_err(|rejection| {
        if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
            ApiError::new(
                StatusCode::PAYLOAD_TOO_LARGE,
                "image_too_large",
                format!("The picture is larger than the {} MiB limit.", MAX_IMAGE_BYTES >> 20),
            )
        } else {
            ApiError::new(StatusCode::BAD_REQUEST, "bad_request", "The picture could not be received.")
        }
    })?;
    if bytes.is_empty() {
        return Err(ApiError::new(StatusCode::BAD_REQUEST, "empty_file", "The picture file is empty."));
    }
    let entry = state.store().get(id)?;
    let info = tokio::task::spawn_blocking(move || {
        let mut doc = entry.lock().unwrap_or_else(|p| p.into_inner());
        doc.document.stage_image(bytes.to_vec())
    })
    .await
    .map_err(|_| internal_error())?
    .map_err(|err| match err {
        DocxError::InvalidEdit(_) => ApiError::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsupported_image",
            "This picture can't be inserted. Use a PNG, JPEG, GIF or BMP file.",
        ),
        other => docx_error(other),
    })?;
    Ok((StatusCode::CREATED, Json(info)))
}

fn stale_revision() -> ApiError {
    ApiError::new(
        StatusCode::CONFLICT,
        "stale_revision",
        "The document changed since this edit was made. Reload it and repeat the change.",
    )
}

async fn undo(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    body: Result<Json<HistoryRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<DocumentResponse>, ApiError> {
    step_history(state, id, body, true).await
}

async fn redo(
    State(state): State<AppState>,
    Path(id): Path<u64>,
    body: Result<Json<HistoryRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<DocumentResponse>, ApiError> {
    step_history(state, id, body, false).await
}

async fn step_history(
    state: AppState,
    id: u64,
    body: Result<Json<HistoryRequest>, axum::extract::rejection::JsonRejection>,
    undo: bool,
) -> Result<Json<DocumentResponse>, ApiError> {
    let Json(request) = body.map_err(|rejection| {
        ApiError::new(StatusCode::BAD_REQUEST, "bad_request", format!("Invalid request: {}", rejection.body_text()))
    })?;
    let entry = state.store().get(id)?;
    let response = tokio::task::spawn_blocking(move || {
        let mut doc = entry.lock().unwrap_or_else(|p| p.into_inner());
        if doc.revision != request.base_revision {
            return Err(stale_revision());
        }
        let doc = &mut *doc;
        let stepped = if undo { doc.history.undo(&mut doc.document) } else { doc.history.redo(&mut doc.document) };
        match stepped.map_err(docx_error)? {
            true => doc.revision += 1,
            false => {
                let (code, message) = if undo {
                    ("nothing_to_undo", "There is nothing to undo.")
                } else {
                    ("nothing_to_redo", "There is nothing to redo.")
                };
                return Err(ApiError::new(StatusCode::CONFLICT, code, message));
            }
        }
        Ok(respond(id, doc))
    })
    .await
    .map_err(|_| internal_error())??;
    Ok(Json(response))
}

/// Image types the browser can display. SVG is excluded: served from this
/// origin it could run script if opened directly.
fn image_mime(part: &str) -> Option<&'static str> {
    let ext = part.rsplit('.').next()?.to_ascii_lowercase();
    Some(match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" | "jpe" => "image/jpeg",
        "gif" => "image/gif",
        "bmp" => "image/bmp",
        "webp" => "image/webp",
        _ => return None,
    })
}

async fn media(State(state): State<AppState>, Path((id, part)): Path<(u64, String)>) -> Result<Response, ApiError> {
    let mime = image_mime(&part).ok_or_else(|| ApiError::not_found("This image type cannot be displayed."))?;
    let entry = state.store().get(id)?;
    let bytes = {
        let doc = entry.lock().unwrap_or_else(|p| p.into_inner());
        doc.document.image_part(&part).map(<[u8]>::to_vec)
    }
    .ok_or_else(|| ApiError::not_found("The document has no such image."))?;
    let mut response = bytes.into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(mime));
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    Ok(response)
}

async fn close(State(state): State<AppState>, Path(id): Path<u64>) -> Result<StatusCode, ApiError> {
    match state.store().documents.remove(&id) {
        Some(_) => Ok(StatusCode::NO_CONTENT),
        None => Err(document_not_found()),
    }
}

fn document_not_found() -> ApiError {
    ApiError::not_found("That document is not open. It may have been closed or the editor was restarted.")
}

fn internal_error() -> ApiError {
    ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "internal_error", "Something went wrong in the editor.")
}

/// Maps engine errors to HTTP errors with user-facing messages.
fn docx_error(err: DocxError) -> ApiError {
    let message = err.to_string();
    match err {
        DocxError::InvalidArchive(_) | DocxError::NotWordDocument(_) | DocxError::MalformedXml { .. } => {
            ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "invalid_document", message)
        }
        DocxError::EncryptedOrLegacy => {
            ApiError::new(StatusCode::UNPROCESSABLE_ENTITY, "unsupported_document", message)
        }
        DocxError::TooLarge(_) => ApiError::new(StatusCode::PAYLOAD_TOO_LARGE, "document_too_large", message),
        DocxError::InvalidEdit(_) => ApiError::new(StatusCode::BAD_REQUEST, "invalid_edit", message),
        DocxError::Write(_) => ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, "save_failed", message),
    }
}

/// Reduces a client-supplied file name to a safe base name with a Word
/// extension. It is only used for display and in `Content-Disposition`.
pub fn sanitize_file_name(raw: &str) -> String {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String =
        base.chars().filter(|c| !c.is_control() && !matches!(c, '"' | '<' | '>' | ':' | '|' | '?' | '*')).collect();
    let mut name: String = cleaned.trim().trim_matches('.').trim().chars().take(200).collect();
    if name.is_empty() {
        return UNTITLED.to_string();
    }
    let lower = name.to_ascii_lowercase();
    if ![".docx", ".docm", ".dotx", ".dotm"].iter().any(|ext| lower.ends_with(ext)) {
        name.push_str(".docx");
    }
    name
}

fn mime_for(name: &str) -> &'static str {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".docm") {
        DOCM_MIME
    } else if lower.ends_with(".dotx") {
        DOTX_MIME
    } else if lower.ends_with(".dotm") {
        DOTM_MIME
    } else {
        DOCX_MIME
    }
}

/// `attachment` with an ASCII fallback name and the UTF-8 name (RFC 6266).
fn content_disposition(name: &str) -> String {
    let ascii: String = name.chars().map(|c| if c.is_ascii() && c != '\\' { c } else { '_' }).collect();
    let encoded: String = name
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect();
    format!("attachment; filename=\"{ascii}\"; filename*=UTF-8''{encoded}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_sanitized() {
        let cases = [
            ("report.docx", "report.docx"),
            ("Report Q3.DOCX", "Report Q3.DOCX"),
            ("C:\\Users\\me\\secret\\plan.docx", "plan.docx"),
            ("../../etc/passwd", "passwd.docx"),
            ("macro.docm", "macro.docm"),
            ("notes", "notes.docx"),
            ("bad\"name<>|?*.docx", "badname.docx"),
            ("line\nbreak.docx", "linebreak.docx"),
            ("", UNTITLED),
            ("   ", UNTITLED),
            ("...", UNTITLED),
            ("résumé.docx", "résumé.docx"),
        ];
        for (input, expected) in cases {
            assert_eq!(sanitize_file_name(input), expected, "{input:?}");
        }
        assert!(sanitize_file_name(&"a".repeat(500)).len() <= 205);
    }

    #[test]
    fn content_types_follow_the_extension() {
        assert_eq!(mime_for("a.docx"), DOCX_MIME);
        assert_eq!(mime_for("a.DOCM"), DOCM_MIME);
        assert_eq!(mime_for("a.dotx"), DOTX_MIME);
        assert_eq!(mime_for("a.dotm"), DOTM_MIME);
    }

    #[test]
    fn content_disposition_handles_unicode() {
        assert_eq!(
            content_disposition("résumé 1.docx"),
            "attachment; filename=\"r_sum_ 1.docx\"; filename*=UTF-8''r%C3%A9sum%C3%A9%201.docx"
        );
    }

    #[test]
    fn only_displayable_images_are_served() {
        assert_eq!(image_mime("word/media/image1.PNG"), Some("image/png"));
        assert_eq!(image_mime("word/media/photo.jpeg"), Some("image/jpeg"));
        assert_eq!(image_mime("word/media/drawing.svg"), None);
        assert_eq!(image_mime("word/media/chart.emf"), None);
        assert_eq!(image_mime("word/document.xml"), None);
    }

    #[test]
    fn engine_errors_map_to_statuses() {
        assert_eq!(docx_error(DocxError::InvalidArchive("x".into())).status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(docx_error(DocxError::EncryptedOrLegacy).code, "unsupported_document");
        assert_eq!(docx_error(DocxError::TooLarge("x".into())).status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(docx_error(DocxError::InvalidEdit("x".into())).status, StatusCode::BAD_REQUEST);
        assert_eq!(docx_error(DocxError::Write("x".into())).status, StatusCode::INTERNAL_SERVER_ERROR);
    }
}
