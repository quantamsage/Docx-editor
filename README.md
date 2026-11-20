# DOCX Editor

A local, web-based editor for Microsoft Word `.docx` documents. A Rust (Axum/Tokio) server runs on your computer and serves a Word-style interface built with plain HTML, CSS and JavaScript. Nothing is sent over the internet.

> **Status: Phase 3 (editing).** You can create a blank document, open existing `.docx` files (button, `Ctrl+O`, or drag and drop), edit their text and paragraphs on a page with the document's real size and margins, and save them again (`Ctrl+S`, `Ctrl+Shift+S` for Save As) without losing content or formatting you didn't touch. Formatting controls, table editing, undo and find & replace come in later phases and stay disabled until then.

## Requirements

- [Rust](https://rustup.rs/) 1.86 or newer (edition 2024).
- A modern browser. Chromium-based browsers (Edge, Chrome) save back to the file you opened; other browsers save through a download.
- Optional, for the frontend and browser tests: [Node.js](https://nodejs.org/) 22 or newer. The application itself does not use Node.

## Running

```sh
cargo run --release
```

The editor starts at <http://localhost:8080> and opens in your default browser. Press `Ctrl+C` to stop it.

| Option | Description |
| --- | --- |
| `-p`, `--port <PORT>` | Port to listen on (default `8080`; `0` picks any free port). Also settable with `DOCX_EDITOR_PORT`. |
| `--no-open` | Don't open the browser automatically. |
| `-h`, `--help` | Show help. |
| `-V`, `--version` | Show the version. |

Set `RUST_LOG=debug` for more detailed logs. The server only listens on `127.0.0.1`. A built binary is self-contained: the interface is embedded in it.

## Opening and saving

- **Open:** choose a `.docx`, `.docm`, `.dotx` or `.dotm` file. The browser reads it and sends it to the local server, which keeps it in memory.
- **Save:** in Edge/Chrome, writes back to the file you opened (or asks where to save a new document). Elsewhere, the document is downloaded.
- **Save As:** asks for a new file name and location.
- Password-protected files and old binary `.doc` files cannot be opened; Word can convert them to `.docx`.

What is preserved: everything. Parts of the file that were not edited are written back byte-for-byte (styles, headers, footers, images, comments, footnotes, numbering, custom XML, settings, …), and inside the main document only edited paragraphs change. See [ARCHITECTURE.md](ARCHITECTURE.md#preservation-guarantees) for details and limitations.

## Editing

- Click in the page and type. Text you type takes the formatting of the text around it; unchanged runs keep their exact formatting.
- **Enter** starts a new paragraph with the same paragraph style and properties (a heading continues as a heading), **Shift+Enter** inserts a line break, **Tab** a tab character.
- **Backspace** at the start of a paragraph joins it to the previous one; **Delete** at the end joins the next one. Selecting across paragraphs and typing or deleting replaces the selection.
- **Paste** inserts plain text; each line becomes a paragraph.
- Images, hyperlinks, page breaks and preserved content are shown as protected objects: you can type around them and delete images or links, but not edit inside them yet. Fields (such as page numbers) and comment markers are always kept. Tables and content controls are shown read-only.
- Changes are applied to the document in the editor as you type. The title bar shows "Unsaved changes" until you save, and closing the tab asks first.

## Tests

```sh
cargo test                     # Rust unit and integration tests (engine and server)
cargo clippy --all-targets     # lints
cargo fmt --check              # formatting
npm test                       # frontend unit tests (Node's built-in test runner; nothing to install)

cargo build --release
npm run test:browser           # drives the real interface in headless Edge/Chrome (types, presses Enter/Backspace/Delete, edits a Word file)
```

Word compatibility checks (Windows with Microsoft Word installed):

```powershell
# Regenerate the Word-made test document (tests/fixtures/word-sample.docx)
./scripts/make-word-fixture.ps1

# Create edited copies with the engine, then check what Word sees in them
cargo run -p docx-engine --example docx_tool -- edit tests/fixtures/word-sample.docx target/edited.docx --text 2 "New text" --align 1 center
./scripts/verify-with-word.ps1 tests/fixtures/word-sample.docx target/edited.docx
```

`docx_tool` can also print a document's structure: `cargo run -p docx-engine --example docx_tool -- inspect <file.docx>`.

See [ARCHITECTURE.md](ARCHITECTURE.md) for how the application is organized.
