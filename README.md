# DOCX Editor

A local, web-based editor for Microsoft Word `.docx` documents. A Rust (Axum/Tokio) server runs on your computer and serves a Word-style interface built with plain HTML, CSS and JavaScript. Nothing is sent over the internet.

> **Status: Phase 2 (document engine).** You can create a blank document, open existing `.docx` files (button, `Ctrl+O`, or drag and drop) and save them again (`Ctrl+S`, `Ctrl+Shift+S` for Save As) without losing their content or formatting. The page shows the document's real page size and margins, a summary of what it contains, and an outline of its headings. Displaying and editing the text comes in Phase 3. Formatting controls stay disabled until their phases.

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

## Tests

```sh
cargo test                     # Rust unit and integration tests (engine and server)
cargo clippy --all-targets     # lints
cargo fmt --check              # formatting
npm test                       # frontend unit tests (Node's built-in test runner; nothing to install)

cargo build --release
npm run test:browser           # drives the real interface in headless Edge/Chrome
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
