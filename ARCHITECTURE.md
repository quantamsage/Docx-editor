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
  src/edit.rs                  editing operations (Splice, Piece, Edit) sent by the editor
  src/document.rs              Document: open, model, edit (splice, apply), save
  src/blank.rs                 template for new documents
  tests/engine.rs              synthetic "hard cases" document: parsing, preservation, edits, errors
  tests/word_fixture.rs        tests against a document written by Microsoft Word
  tests/splice.rs              typing, Enter, joining, cross-paragraph deletes, paste, sections, atomicity
  examples/docx_tool.rs        inspect/edit files from the command line
crates/editor-server/
  src/main.rs                  CLI entry: parse args → bind 127.0.0.1:<port> → open browser → serve
  src/app.rs                   router: page, static assets, /api routes, fallbacks
  src/documents.rs             open-document store and /api/documents endpoints
  src/assets.rs                frontend files embedded with include_bytes!
  src/security.rs              loopback Host check, cross-origin check, security headers
  src/config.rs, error.rs      CLI configuration; ApiError → JSON
  tests/http.rs, documents.rs  router and API integration tests
  tests/edits.rs               edit and image endpoints
web/
  index.html, css/app.css      application shell
  js/app.js                    entry point: toolbar, shortcuts, drag & drop, rendering
  js/api.js                    fetch wrapper for the JSON API
  js/files.js                  file pickers, saving to handles, downloads
  js/documents.js              interpreting the model: page geometry, summary, outline
  js/status.js                 status bar: word count, page position, connection monitor
  js/editor/content.js         model ⇄ editable segments ⇄ splice pieces (no DOM; unit-tested)
  js/editor/render.js          model → editable page (paragraphs, runs, objects, read-only tables)
  js/editor/dom.js             reading the page: segments, caret offsets, selection
  js/editor/editor.js          input handling, background sync, structural edits, recovery
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
- Editing replaces only the runs whose text changed; new text copies the formatting (`w:rPr`) of the run it was typed into. Markers (bookmarks, comment ranges, permission ranges, proofing marks) keep their order, but new text is placed where the first removed run was, so a bookmark or comment range next to edited text can end up just before or after the new text rather than around it.
- Deleting a paragraph that carries a section break is refused (it would merge two sections' page layouts).
- Content controls (`w:sdt`), tracked changes, fields and similar constructs are preserved and their text is shown in the model, but they are not editable.
- Documents that bind the WordprocessingML namespace as the default namespace are supported; the engine declares a `w:` prefix on the root when it first needs to write attributes. Strict OOXML documents are read; their edits are untested against Word.
- UTF-16 encoded parts are rejected; Word always writes UTF-8.
- Documents are kept in memory while open (at most 32 at a time, 100 MiB per upload).

## Editing

### Edit operations

Every text edit is a **splice** (`docx_engine::Splice`): paragraphs `first..=last` are replaced by a list of pieces:

- `keep {block, inline}` keeps an existing inline element (run, hyperlink, image, field run, preserved element) exactly as it is;
- `text {text, like?}` adds a new run, copying the run properties of the run `like` (`\t` and `\n` become `w:tab` and `w:br`);
- `break` ends the paragraph and starts a new one.

Inlines are addressed by their index in the model (`blocks[b].inlines[i]`), which counts the paragraph's children except `w:pPr` and markers. The engine walks the original children of the affected paragraphs in order: kept inlines and markers stay in order, unkept inlines are removed, and new text and breaks are inserted at the first removed inline (where the edit happened) or before the next kept one. The first resulting paragraph reuses the first paragraph's element and properties; paragraphs created by a break copy those properties (without paraIds). A section break carried by the last paragraph moves to the last resulting paragraph; a splice that would remove a section break in between is refused. Splices require every block in the range to be a paragraph, so selections across tables are refused.

Typing (one paragraph), Enter, Shift+Enter, joining paragraphs with Backspace/Delete, deleting or typing over a selection across paragraphs, and pasting are all splices, so one tested operation covers them. `Document::apply` runs a batch atomically (the XML tree is restored if any edit fails).

### Editor ⇄ server

The editor renders paragraphs as `<p data-block>`, text runs as `<span data-kind="run" data-i>` and other inlines as non-editable `<span data-kind="object" data-i>`.

- **Inside a paragraph** the browser handles typing and deleting. The paragraph is marked dirty; 400 ms after typing pauses, the editor reads it back into segments (`dom.js`), turns them into pieces (`content.js`: text equal to its run's text is kept, changed text becomes new text like its run), and sends one splice per dirty paragraph. The page is not re-rendered: its `data-i` labels are remapped to the new model's indices, so the caret never moves and text typed while the request was out is sent next time.
- **Structural edits** (Enter, Shift+Enter, Backspace/Delete at a paragraph edge, typing/deleting over a multi-paragraph selection, paste) are intercepted in `beforeinput`, sent as one splice after pending typing, and the page is re-rendered from the server's model with the caret at the computed position. Input arriving while one is in flight is queued and replayed.
- Invisible objects (field codes, comment references) are "sticky": the editor always keeps them, even when the browser deleted them or they are inside a deleted range, so fields are never left half-deleted.
- Requests run strictly in order and name the document's **revision**; the server refuses edits based on an older revision (`409 stale_revision`). If any edit fails, the editor reloads the document from the server and shows why.
- Undo/redo, formatting commands, drag-and-drop of content and rich paste are blocked until their phases.

### Limitations (Phase 3)

- Pages are one continuous sheet with dashed lines at each page height; real pagination (where Word would break pages) comes with page layout (Phase 5). The page count in the status bar is estimated from the height.
- Tables, content controls and text inside hyperlinks are shown but not editable yet; list numbering is shown as a bullet.
- Each sync returns the whole document model, which is fine for typical documents but grows with very large ones.
- Floating images are shown inline.

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
| `POST /api/documents/new` | Create a blank document → `{id, name, modified, revision, model}`. |
| `POST /api/documents/open?name=…` | Body: the `.docx` bytes. Opens it → `{id, name, modified, model}`. |
| `GET /api/documents/{id}` | The document's current model. |
| `POST /api/documents/{id}/edits` | Body `{base_revision, edits: [...]}`: applies the edits atomically → the updated document. `409 stale_revision` if the revision is outdated, `400 invalid_edit` if an edit is invalid (nothing is changed). |
| `GET /api/documents/{id}/media/{part}` | An image part the document references (PNG, JPEG, GIF, BMP, WebP only), for display. |
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
