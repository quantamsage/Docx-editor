# Architecture

## Overview

```
 Browser (HTML/CSS/vanilla JS)  ──HTTP/JSON on 127.0.0.1──▶  editor-server (Axum/Tokio)  ──Rust calls──▶  docx-engine
```

- **Frontend** (`web/`): the editing interface. Plain ES modules, no framework and no build step. Logic that can be separated from the DOM lives in small modules that are unit-tested with Node's test runner.
- **Backend** (`crates/editor-server`): a thin HTTP layer. It serves the embedded frontend, keeps open documents in memory, and exposes a JSON API under `/api`.
- **Document engine** (`crates/docx-engine`): DOCX reading, the document model, editing operations and writing. A plain library with no HTTP or async dependency.
- **File management**: the browser reads and writes files on disk (file picker, File System Access API, or downloads). The server never opens paths on its own; it only receives bytes the user chose to open and returns bytes to save.

## Layout

```
Cargo.toml                     workspace (shared dependency versions)
crates/docx-engine/
  src/package.rs               OPC package: safe ZIP reading, lossless writing
  src/xml.rs                   lossless, namespace-aware XML tree
  src/opc.rs                   content types, relationships, target resolution
  src/read.rs                  builds the model from document.xml and styles.xml
  src/model.rs                 serializable read model (blocks, runs, tables, section, styles)
  src/props.rs                 schema-ordered property insertion (w:pPr)
  src/document.rs              Document: open, model, edit, save
  src/blank.rs                 template for new documents
  tests/engine.rs              synthetic "hard cases" document: parsing, preservation, edits, errors
  tests/word_fixture.rs        tests against a document written by Microsoft Word
  examples/docx_tool.rs        inspect/edit files from the command line
crates/editor-server/
  src/main.rs                  CLI entry: parse args → bind 127.0.0.1:<port> → open browser → serve
  src/app.rs                   router: page, static assets, /api routes, fallbacks
  src/documents.rs             open-document store and /api/documents endpoints
  src/assets.rs                frontend files embedded with include_bytes!
  src/security.rs              loopback Host check, cross-origin check, security headers
  src/config.rs, error.rs      CLI configuration; ApiError → JSON
  tests/http.rs, documents.rs  router and API integration tests
web/
  index.html, css/app.css      application shell
  js/app.js                    entry point: toolbar, shortcuts, drag & drop, rendering
  js/api.js                    fetch wrapper for the JSON API
  js/files.js                  file pickers, saving to handles, downloads
  js/documents.js              interpreting the model: page geometry, summary, outline
  js/status.js                 status bar: word count, page position, connection monitor
tests/frontend/                Node test-runner tests for web/js modules
tests/browser/smoke.mjs        headless-browser test of the real interface
tests/fixtures/word-sample.docx  document produced by Microsoft Word (see scripts/)
scripts/                       Word fixture generator and Word verification (PowerShell)
```

## Choice of DOCX library

Existing Rust crates were evaluated before writing the engine:

| Crate | What it does | Why it was not used as the engine |
| --- | --- | --- |
| `docx-rs` | Builds documents; can read into its own model | Reading maps into its model and writing regenerates the XML from that model, so elements it does not model (content controls, fields, comments' anchors, `mc:AlternateContent`, vendor extensions) are dropped or rewritten. |
| `docx` | Older parse/generate library | Same model-and-regenerate approach; limited coverage. |
| `rust-docx`, `duke-pages` and similar | Parse into a shared model and serialize | Same approach: designed for conversion, not lossless editing. |

The spec's central requirement is that editing a small part of a document leaves everything else intact. A model-and-regenerate library cannot guarantee that. The engine therefore works directly on the package and XML, using two small, well-maintained crates:

- **`zip`** (deflate only, no encryption or other codecs) for the archive.
- **`quick-xml`** for XML tokenizing; the tree is built on top of it.

## Preservation strategy

1. **Package level.** All parts are read into memory with their names, order, compression method and timestamps. On save, every part is written back in the same order. A part that was not edited is written from its original bytes, so styles, numbering, headers, footers, footnotes, comments, images, embedded objects, custom XML, settings and any unknown parts are untouched.
2. **XML level.** `word/document.xml` is parsed into a lossless tree. Each element keeps the exact text of its original start tag (attribute order, quotes, whitespace, namespace declarations); text keeps its original escaping; comments, processing instructions and the XML declaration are kept verbatim. Serializing an unedited tree reproduces the input byte for byte (tested). If nothing in the document was edited, the original bytes are written without serializing at all.
3. **Edit level.** Edits change only the elements they target. Paragraph properties are set by inserting the property element at its position in the OOXML schema sequence (`CT_PPr`), immediately before the next known sibling that must follow it; unknown siblings (extensions, alternate content) are never moved or removed. Text edits replace only runs that hold plain text (text, tabs, breaks) and keep the formatting of the first such run; runs with images, fields, footnote or comment references, and hyperlinks, bookmarks, comment ranges and other markers stay in place.
4. **Read model.** The model sent to the frontend is derived from the tree. Content the engine does not interpret appears as `other` items (block, inline or run level) with their visible text, so the interface can show that something is there. The model is never written back; the tree is.

### Preservation guarantees

- Unedited documents: every part's content is identical after saving (`unedited_save_is_identical_part_by_part`, `resaving_words_output_changes_nothing`).
- Edited documents: all parts except the main document part are byte-identical; in the main part, markup outside the edited elements is byte-identical (`editing_text_keeps_everything_else_intact`).
- Microsoft Word opens the edited files without a repair prompt and still finds the comment, footnote, picture, table, hyperlink, header and footer field (`scripts/verify-with-word.ps1`, run during development).

### Known limitations

- The ZIP container is rebuilt on save: part contents are identical, but compressed sizes and archive metadata may differ from the original file (the archive is written by the `zip` crate's deflate implementation).
- Paragraph-level edits replace the text of all plain runs in the paragraph with a single run that has the first run's formatting. Character-level edits that keep mixed formatting arrive with the editing surface (Phase 3).
- Deleting a paragraph that carries a section break is refused (it would merge two sections' page layouts).
- Content controls (`w:sdt`), tracked changes, fields and similar constructs are preserved and their text is shown in the model, but they are not editable.
- Documents that bind the WordprocessingML namespace as the default namespace are supported; the engine declares a `w:` prefix on the root when it first needs to write attributes. Strict OOXML documents are read; their edits are untested against Word.
- UTF-16 encoded parts are rejected; Word always writes UTF-8.
- Documents are kept in memory while open (at most 32 at a time, 100 MiB per upload).

## Safety when opening files

- Archives are read entirely in memory and never extracted to disk.
- Limits: 10,000 entries, 256 MiB per part, 512 MiB in total, and a compression ratio of at most 200:1 for parts over 1 MiB (ZIP bombs). Declared sizes are checked, and actual reads are capped regardless.
- Part names with `..`, `.`, empty segments, backslashes, control characters or duplicates (case-insensitive) are rejected. Relationship targets that would leave the package resolve to nothing.
- XML with a DOCTYPE is rejected (no entity expansion). Malformed main-document XML is an error; a damaged styles or relationships part is reported as a warning and kept unchanged.
- OLE compound files (password-protected documents and legacy `.doc`) and encrypted ZIP entries are reported clearly instead of being misread.

## Frontend ↔ backend communication

| Endpoint | Purpose |
| --- | --- |
| `GET /api/health` | Liveness and version (status bar). |
| `POST /api/documents/new` | Create a blank document → `{id, name, modified, model}`. |
| `POST /api/documents/open?name=…` | Body: the `.docx` bytes. Opens it → `{id, name, modified, model}`. |
| `GET /api/documents/{id}` | The document's current model. |
| `GET /api/documents/{id}/download[?name=…]` | The saved `.docx`; `name` renames the document (Save As). |
| `DELETE /api/documents/{id}` | Close the document. |

Errors are `{"error": {"code", "message"}}` with an HTTP status: `invalid_document`/`unsupported_document` (422), `document_too_large` (413), `empty_file` (400), `too_many_documents` (409), `not_found` (404). Parsing and serialization run on Tokio's blocking pool.

The model is JSON: `blocks` (paragraphs with style, alignment, indentation, spacing, numbering and inlines; tables with rows, cells, spans and vertical merges; `other` blocks), `section` (page size, orientation, margins in twips), `styles`, `headers`/`footers`, `stats` and `warnings`. Empty fields are omitted.

## Local-only security

- The server binds to `127.0.0.1` only.
- **Host check:** requests whose `Host` is not `localhost`, `127.0.0.1` or `[::1]` are rejected (DNS rebinding).
- **Origin check:** state-changing requests must come from a loopback origin; foreign `Origin` or `Sec-Fetch-Site: cross-site` is rejected.
- **Headers:** strict Content-Security-Policy (`default-src 'self'`, no inline scripts or styles, nothing from the network), `nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer`, same-origin COOP/CORP. Downloads are `no-store`.
- No telemetry, no external requests, no accounts. File names from the browser are reduced to a safe base name and only used for display and `Content-Disposition`.

## Decisions

- **Workspace with separate crates** so the engine cannot depend on HTTP types.
- **Own engine on `zip` + `quick-xml`** for lossless round trips (see above).
- **Browser-side file access.** The server never touches user paths, which keeps "only files the user selects" true by construction; Chromium's File System Access API gives real in-place saving.
- **Embedded assets via `include_bytes!`**: one self-contained binary and an explicit list of served files.
- **No CLI parsing crate; hand-rolled browser launch.** Small surface, few dependencies.
- **Testing against real Word output.** `tests/fixtures/word-sample.docx` is generated by Word through `scripts/make-word-fixture.ps1` (personal metadata is replaced with neutral values), and edited copies are checked in Word with `scripts/verify-with-word.ps1`.
- **Frontend tests use Node's built-in runner** without npm packages; `tests/browser/smoke.mjs` drives a headless Chromium browser through the DevTools protocol using Node's built-in WebSocket.
