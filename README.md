# DOCX Editor

A local, web-based editor for Microsoft Word `.docx` documents. A Rust (Axum/Tokio) server runs on your computer and serves a Word-style interface built with plain HTML, CSS and JavaScript. Nothing is sent over the internet.

> **Status: Phase 5 (advanced document features).** You can create a blank document, open existing `.docx` files (button, `Ctrl+O`, or drag and drop), edit and format their text and paragraphs, edit tables, insert and resize pictures, edit headers and footers, number pages, change the page setup and insert page breaks, and save the document again (`Ctrl+S`, `Ctrl+Shift+S` for Save As) without losing content or formatting you didn't touch. Undo, find & replace and draft recovery come in Phase 6.

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
- Pictures, hyperlinks, page breaks, fields and preserved content are shown as protected objects: you can type around them and delete pictures, links and page breaks, but not edit inside them. Field codes (such as page numbers) and comment markers are always kept. Content controls and tables nested inside tables are shown read-only.
- Changes are applied to the document in the editor as you type. The title bar shows "Unsaved changes" until you save, and closing the tab asks first.

## Formatting

The formatting toolbar shows the formatting at the cursor and applies changes to the selection (or, for character formatting with nothing selected, to the word at the cursor):

| Control | Shortcut |
| --- | --- |
| Paragraph style (Normal, Title, Heading 1–3 and the document's own styles) | `Ctrl+Alt+1/2/3` headings, `Ctrl+Alt+0` Normal |
| Font and size | `Ctrl+]` / `Ctrl+[` larger / smaller |
| Bold, italic, underline, strikethrough | `Ctrl+B`, `Ctrl+I`, `Ctrl+U`, `Ctrl+Shift+X` |
| Text colour and highlight colour | |
| Align left, centre, right, justify | `Ctrl+Shift+L/E/R/J` |
| Bulleted and numbered lists | `Ctrl+Shift+8`, `Ctrl+Shift+7` |
| Decrease / increase indent (changes the list level in lists) | `Ctrl+Shift+M`, `Ctrl+M` |
| Line spacing and paragraph spacing | |

Heading and title styles a document doesn't define are added to it with Word's standard look. Lists reuse the document's own bullet or numbered list where it has one; a document without lists gets a numbering definition added. Formatting is written as ordinary Word properties, so Word shows it the same way.

## Tables, pictures and page layout

The **Insert** group of the toolbar:

| Button | What it does |
| --- | --- |
| Table | Inserts a table (choose rows and columns) at the cursor, spread over the text width with single borders. |
| Picture | Inserts a PNG, JPEG, GIF or BMP picture at the cursor, scaled down to the text width if needed. You can also drop a picture file on the page. |
| Page break | Starts a new page at the cursor (`Ctrl+Enter`). |
| Header / Footer | Puts the cursor in the header or footer, adding an empty one if the document has none. Double-clicking the empty top or bottom margin does the same. |
| Page numbers | Adds the page number (optionally "Page 1 of 3") to the footer or header, left, centred or right, and can set the first page's number. |
| Page setup | Paper size (A4, Letter, Legal, A5, A3), orientation and margins (Normal, Narrow, Moderate, Wide or custom, in cm) for the whole document. |

**Tables:** click in a cell to type in it; Enter starts a new paragraph in the cell, and the formatting toolbar works there as everywhere else. While the cursor is in a table the **Table** toolbar appears: insert a row above or below, a column left or right, delete the row, column or table, show or hide the borders, align the cell's text to the top, middle or bottom, and shade the cell. Column widths, merged cells, shading and alignment from Word are shown; inserting and deleting columns keeps the table's width.

**Pictures:** click a picture to select it; drag the square handle at its corner to resize it (its proportions are kept), or press Delete to remove it.

**Headers and footers** are shown in the top and bottom margins and edited like the main text (they are dimmed until you point at them). The editor shows the document's main header and footer; first-page and even-page headers are kept but not shown.

## Tests

```sh
cargo test                     # Rust unit and integration tests (engine and server)
cargo clippy --all-targets     # lints
cargo fmt --check              # formatting
npm test                       # frontend unit tests (Node's built-in test runner; nothing to install)

cargo build --release
npm run test:browser           # drives the real interface in headless Edge/Chrome (typing, formatting, tables, pictures, headers, page setup, editing a Word file)
```

Word compatibility checks (Windows with Microsoft Word installed):

```powershell
# Regenerate the Word-made test document (tests/fixtures/word-sample.docx)
./scripts/make-word-fixture.ps1

# Create edited copies with the engine, then check what Word sees in them
cargo run -p docx-engine --example docx_tool -- edit tests/fixtures/word-sample.docx target/edited.docx --text 2 "New text" --align 1 center
./scripts/verify-with-word.ps1 tests/fixtures/word-sample.docx target/edited.docx
```

`docx_tool` can also print a document's structure (`inspect <file.docx>`), check that its XML is in schema order (`check <file.docx>`), and apply edits as the editor sends them (`edit <in> <out> --stage picture.png --json edits.json`).

The browser test can save screenshots (`SCREENSHOT_DIR=<folder>`) and the documents it edited (`SAVE_DIR=<folder>`), for checking them with `docx_tool check` and `scripts/verify-with-word.ps1`.

See [ARCHITECTURE.md](ARCHITECTURE.md) for how the application is organized.
