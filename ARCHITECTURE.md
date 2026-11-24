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
  src/format.rs                run and paragraph formats, applied in schema order
  src/numbering.rs             list definitions: reading, reusing and creating them
  src/table.rs                 table edits: new tables, rows, columns, borders, cell properties
  src/media.rs                 pictures: recognizing uploads, DrawingML markup, resizing
  src/check.rs                 schema-order and structure checks Word is strict about
  src/history.rs               undo and redo: bounded history of document snapshots
  src/document.rs              Document: open, model, containers, edits, parts and relationships, save
  src/blank.rs                 template for new documents
  tests/engine.rs              synthetic "hard cases" document: parsing, preservation, edits, errors
  tests/word_fixture.rs        tests against a document written by Microsoft Word
  tests/splice.rs              typing, Enter, joining, cross-paragraph deletes, paste, sections, atomicity
  tests/formatting.rs          run and paragraph formats, styles added on demand, lists
  tests/tables.rs              cell editing, inserting tables, rows, columns, borders, cell properties
  tests/pictures.rs            staging, inserting, reusing, resizing and removing pictures
  tests/layout.rs              headers and footers, page numbers, page setup, page breaks
  tests/history.rs             undo and redo of every kind of edit, grouping, bounds
  tests/preservation.rs        every edit on both test documents: untouched parts and blocks, constructs, re-saving
  examples/docx_tool.rs        inspect/edit files from the command line
crates/editor-server/
  src/main.rs                  CLI entry: parse args → bind 127.0.0.1:<port> → open browser → serve
  src/app.rs                   router: page, static assets, /api routes, fallbacks
  src/documents.rs             open-document store and /api/documents endpoints
  src/assets.rs                frontend files embedded with include_bytes!
  src/security.rs              loopback Host check, cross-origin check, security headers
  src/config.rs, error.rs      CLI configuration; ApiError → JSON
  tests/http.rs, documents.rs  router and API integration tests
  tests/edits.rs               edit, picture upload and image endpoints
web/
  index.html, css/app.css      application shell
  js/app.js                    entry point: toolbar, shortcuts, drag & drop, rendering
  js/api.js                    fetch wrapper for the JSON API
  js/files.js                  file pickers, saving to handles, downloads, save error messages
  js/drafts.js                 local drafts in IndexedDB, Web Locks, which drafts to offer
  js/documents.js              interpreting the model: page geometry, summary, outline
  js/layout.js                 paper sizes, margin presets, cm conversion, picture sizes
  js/keyboard.js               toolbar keyboard navigation (roving Tab stop), F6 regions, Alt+F10
  js/announce.js               screen-reader announcements through live regions
  js/status.js                 status bar: word count, page estimate, connection monitor
  js/editor/content.js         model ⇄ editable segments ⇄ splice pieces (no DOM; unit-tested)
  js/editor/containers.js      container keys: body, table cells, header, footer (no DOM)
  js/editor/tables.js          table commands → edits (no DOM)
  js/editor/render.js          model → editable page (paragraphs, runs, objects, tables, header, footer)
  js/editor/dom.js             reading the page: segments, caret offsets, selection
  js/editor/editor.js          input handling, background sync, structural and formatting edits, recovery
  js/editor/toolbar.js         formatting toolbar: state from the selection, commands, shortcuts
  js/editor/insert.js          Insert group, dialogs, table toolbar
  js/editor/images.js          selecting pictures and resizing them with a handle
  js/editor/find.js            find matches and replacement splices (no DOM)
  js/editor/findbar.js         the find and replace bar, match highlighting
tests/frontend/                Node test-runner tests for web/js modules
tests/browser/harness.mjs      starts the editor and headless Edge/Chrome; DevTools protocol client
tests/browser/smoke.mjs        every feature through the real interface (new document and Word document)
tests/browser/accessibility.mjs  keyboard only, focus, dialogs, announcements, accessibility tree, contrast, phone layout
tests/browser/perf.mjs         timings on a 330-page document (built by large-document.mjs)
tests/browser/screenshots.mjs  the README screenshots (docs/screenshots/)
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

## Preservation

1. **Package level.** All parts are read into memory with their names, order, compression method and timestamps. On save, every part is written back in the same order. A part that was not edited is written from its original bytes, so styles, numbering, headers, footers, footnotes, comments, images, embedded objects, custom XML, settings and any unknown parts are untouched.
2. **XML level.** `word/document.xml` is parsed into a lossless tree. Each element keeps the exact text of its original start tag (attribute order, quotes, whitespace, namespace declarations); text keeps its original escaping; comments, processing instructions and the XML declaration are kept verbatim. Serializing an unedited tree reproduces the input byte for byte (tested). If nothing in the document was edited, the original bytes are written without serializing at all.
3. **Edit level.** Edits change only the elements they target. Paragraph properties are set by inserting the property element at its position in the OOXML schema sequence (`CT_PPr`), immediately before the next known sibling that must follow it; unknown siblings (extensions, alternate content) are never moved or removed. Text edits replace only runs that hold plain text (text, tabs, breaks) and keep the formatting of the first such run; runs with images, fields, footnote or comment references, and hyperlinks, bookmarks, comment ranges and other markers stay in place.
4. **Read model.** The model sent to the frontend is derived from the tree. Content the engine does not interpret appears as `other` items (block, inline or run level) with their visible text, so the interface can show that something is there. The model is never written back; the tree is.

### Preservation guarantees

- Unedited documents: every part's content is identical after saving (`unedited_save_is_identical_part_by_part`, `resaving_words_output_changes_nothing`).
- Edited documents: only the parts an edit needs are rewritten (the main document; the edited header or footer; and, when an edit adds something, the style sheet, numbering, relationships, content types and new pictures or headers). All other parts are byte-identical, and in rewritten parts the markup outside the edited elements is byte-identical (`editing_text_keeps_everything_else_intact`, `changed_parts` checks in `tables.rs`, `pictures.rs` and `layout.rs`).
- Microsoft Word opens the edited files without a repair prompt and still finds the comment, footnote, picture, table, hyperlink, header and footer field (`scripts/verify-with-word.ps1`, run during development).

## Editing

### Edit operations

Every text edit is a **splice** (`docx_engine::Splice`): paragraphs `first..=last` are replaced by a list of pieces:

- `keep {block, inline}` keeps an existing inline element (run, hyperlink, image, field run, preserved element) exactly as it is;
- `text {text, like?}` adds a new run, copying the run properties of the run `like` (`\t` and `\n` become `w:tab` and `w:br`);
- `break` ends the paragraph and starts a new one.

Inlines are addressed by their index in the model (`blocks[b].inlines[i]`), which counts the paragraph's children except `w:pPr` and markers. The engine walks the original children of the affected paragraphs in order: kept inlines and markers stay in order, unkept inlines are removed, and new text and breaks are inserted at the first removed inline (where the edit happened) or before the next kept one. The first resulting paragraph reuses the first paragraph's element and properties; paragraphs created by a break copy those properties (without paraIds). A section break carried by the last paragraph moves to the last resulting paragraph; a splice that would remove a section break in between is refused. Splices require every block in the range to be a paragraph, so selections across tables are refused.

Typing (one paragraph), Enter, Shift+Enter, joining paragraphs with Backspace/Delete, deleting or typing over a selection across paragraphs, and pasting are all splices, so one tested operation covers them. `Document::apply` runs a batch atomically: it takes a snapshot first (the edited parts serialized, see *Undo and redo*) and restores it if any edit fails.

### Editor ⇄ server

The editor renders paragraphs as `<p data-block>`, text runs as `<span data-kind="run" data-i>` and other inlines as non-editable `<span data-kind="object" data-i>`.

- **Inside a paragraph** the browser handles typing and deleting. The paragraph is marked dirty; 400 ms after typing pauses, the editor reads it back into segments (`dom.js`), turns them into pieces (`content.js`: text equal to its run's text is kept, changed text becomes new text like its run), and sends one splice per dirty paragraph. The page is not re-rendered: its `data-i` labels are remapped to the new model's indices, so the caret never moves and text typed while the request was out is sent next time.
- **Structural edits** (Enter, Shift+Enter, Backspace/Delete at a paragraph edge, typing/deleting over a multi-paragraph selection, paste) are intercepted in `beforeinput`, sent as one splice after pending typing, and the page is re-rendered from the server's model with the caret at the computed position. Input arriving while one is in flight is queued and replayed.
- Invisible objects (field codes, comment references) are "sticky": the editor always keeps them, even when the browser deleted them or they are inside a deleted range, so fields are never left half-deleted.
- Requests run strictly in order and name the document's **revision**; the server refuses edits based on an older revision (`409 stale_revision`). If any edit fails, the editor reloads the document from the server and shows why.
- Drag-and-drop of content and rich paste are blocked (pasting inserts plain text).

### Containers

Paragraphs live in **containers**: the document body, a cell of a table, or a header or footer part (or a cell of a table in one). `Splice` and `FormatParagraphs` take a `container` (`{part?, cell?: {table, row, cell}}`; absent = the body), and block and inline indices count within it, so typing, Enter, joining, pasting and formatting in table cells and headers run through the same tested splice code. The engine resolves the container to the element whose children are its blocks (`w:body`, `w:tc`, `w:hdr`/`w:ftr`); a header or footer part is parsed into `Document.edited` on its first edit, so it alone is re-serialized. Edits can't span containers.

On the page every editable paragraph carries `data-block` and, outside the body, `data-container` with the container's key (`containers.js`). The header and footer are separate editing hosts (`#doc-header`, `#doc-footer`) around the body (`#doc-content`); the editor listens on all three, and the browser doesn't let a selection run from one host into another.

## Formatting

**Character formatting** travels with splices: a `text` piece may carry a `format` (`bold`, `italic`, `underline`, `strike`, `font`, `size` in half-points, `color`, `highlight`). The engine copies the `like` run's `w:rPr` and applies only the named changes (`null` removes a property so the style applies; `false` writes an explicit off, e.g. `<w:b w:val="0"/>`, which can switch off a style's bold). Theme font and colour attributes are removed when an explicit font or colour is set, since they would override it. The changed `w:rPr` is put in the `CT_RPr` schema order; unknown children (language, kerning, extensions) are kept next to the property they followed. The frontend builds these splices with `content.formatRange`: text outside the selection keeps its runs (whole runs are kept untouched), selected text is rewritten with its own formatting plus the change, objects are kept.

**Paragraph formatting** is the `format_paragraphs` edit (`style_id`, `alignment`, `indent`, `spacing`, `list`), applied to every paragraph in a range by setting the corresponding `w:pPr` children in schema order. Absent fields are unchanged; `null` removes the property.

- **Styles:** applying a style the document doesn't define is allowed for the built-in set (Title, Heading 1–3, List Paragraph, …): the definition from the blank template is appended to `styles.xml` (only when it uses the usual `w:` prefix). Other unknown styles are refused.
- **Lists:** `numbering.find_or_create` reuses the first `w:num` whose level 0 is a bullet (or decimal), or appends a 9-level `w:abstractNum` (keeping all abstractNums before the first `w:num`, as the schema requires) and a `w:num`. A document without a numbering part gets `word/numbering.xml`, a relationship and a content-type override.
- Secondary parts changed this way (`styles.xml`, `numbering.xml`, relationships, `[Content_Types].xml`) are kept as parsed trees in `Document.edited`, written on save (new parts appended to the archive), and rolled back with the main part when a batch fails. All other parts stay byte-identical.

**Display:** the model includes `lists` (each `w:num` with its levels' format, text, start and indent). `content.listMarkers` numbers list paragraphs per list and level (restarting deeper levels, formatting `%1.%2.` with each level's numbering format, letters and roman numerals) and maps Word's symbol-font bullets (e.g. U+F0B7) to Unicode. The marker hangs in the paragraph's indent.

**Toolbar:** `toolbar.js` reads the formatting at the selection (`editor.currentFormat()`: the run before the cursor, the paragraph, its list kind) on every selection change and sets button states and menus. Buttons act on mousedown without taking focus, so the selection is kept; after an edit the page is re-rendered and the selection restored. Shortcuts follow common editor conventions (some Word shortcuts such as `Ctrl+L` are reserved by browsers).

## Tables, pictures, headers and page layout

### Tables

`table.rs` works on a `w:tbl` element. Edits: `insert_table {at, rows, columns}`, `insert_row {table, row, after}`, `delete_row`, `insert_column {table, row, cell, after}`, `delete_column`, `set_table_borders {table, borders}` and `format_cell {table, row, cell, format: {vertical_align, shading}}`; a table is deleted with `delete_block`.

- **New tables** get `tblW` auto, explicit single `tblBorders`, the Table Grid style when the document defines it, a `tblGrid` dividing the last section's text width, and a `tcW` per cell. Empty paragraphs are added around a new table where it would touch another table (Word merges adjacent tables) or end the container.
- **Rows** copy the reference row's `tblPrEx`, `trPr` (without header-row and change-tracking marks) and cell properties (without spans, merges and change marks); each new cell gets an empty paragraph with the reference cell's paragraph properties. A new row inside a vertical merge continues it. Deleting a row that starts a merge makes the next row's cell restart it; a table keeps at least one row.
- **Columns** work on the grid: the column is inserted next to the cursor's cell, and a cell spanning the insertion point is widened (`gridSpan`) instead of split. Deleting narrows spanning cells and removes the others. The new grid is scaled so the table keeps its total width, and every `tcW` in twips is recomputed from the columns it covers.
- Tables whose structure the editor can't follow safely (rows shifted with `gridBefore`/`gridAfter`, rows or cells wrapped in content controls, no grid) are refused for row and column edits with a message; their text can still be edited.
- Properties are written in `CT_TblPr`, `CT_TcPr` and `CT_TblBorders` order; unknown children are kept.

### Pictures

Inserting a picture takes two steps, so that the document changes in one atomic edit:

1. `POST /api/documents/{id}/images` stages the file in the engine (`Document::stage_image`). It is recognized by its content (PNG, JPEG, GIF, BMP; the size in pixels is read from the file header), never by its name; SVG and other formats are refused. Staged pictures are not part of the document.
2. A splice piece `{type: "image", image, width_emu, height_emu, description}` adds the picture: the part `word/media/imageN.<ext>` (once, however often it is inserted), a `Default` content type for its extension if missing, an image relationship from the part that shows it (the main document or a header or footer), and an inline `w:drawing` written the way Word writes it, with a `wp:docPr` id unique in the document. DrawingML namespaces are declared on the elements that use them, as Word itself does.

`resize_image {block, inline, width_emu, height_emu}` changes `wp:extent` and the picture's `a:ext` only. Pictures are deleted like any object (a splice that doesn't keep them); on save, a picture the editor added that nothing references any more is left out of the file, with its relationship. Pictures that came with the document are always kept.

### Headers, footers and page numbers

The model's `header` and `footer` are the default header and footer of the last section (its own reference, or the one inherited from an earlier section), with their part name and blocks. `add_header_footer {kind}` creates one when the document has none: a `word/headerN.xml` part (with the Header or Footer style, added to the style sheet if needed), its content-type override and relationship, and a `w:headerReference w:type="default"` in every section that has none, inserted with the other references at the start of `w:sectPr`. `insert_page_numbers {kind, alignment, with_total}` adds a paragraph with `PAGE` (and `NUMPAGES`) simple fields, filling the empty paragraph of a new header or footer; it is refused when the part already has a `PAGE` field.

### Page setup and page breaks

`format_section {size, landscape, margins, page_number_start}` changes every section (`w:sectPr`) of the document, creating the body's final section properties if missing: `w:pgSz` (width and height swapped for the orientation, `w:orient`), `w:pgMar` (all seven required attributes when it is created) and `w:pgNumType/@w:start`, in `CT_SectPr` order. Margins that leave less than a quarter inch for text are refused. A page break is a splice piece `{type: "page_break"}` (a run with `<w:br w:type="page"/>`); `Ctrl+Enter` inserts it followed by a paragraph break, as Word does.

### Checking the output

`check.rs` reports what Word is strict about: property children out of schema order or repeated (`pPr`, `rPr`, `sectPr`, `tblPr`, `tcPr`, `tblBorders`, `tbl`, `tr`), cell, paragraph and run properties that are not first, cells that don't end with a paragraph, and section properties that are not last in the body. The engine tests run it on every document they save, and `docx_tool check` runs it on a file. Documents edited by the engine, through the API and in the browser were also opened in Word with `scripts/verify-with-word.ps1` (tables with inserted and deleted rows and columns, cell shading and alignment, pictures, header text with a picture, "Page 3 of 4" with a page number start, landscape Letter).

## Saving and reliability

### Reliable export

`Document::save` writes only what changed (see the preservation strategy), then reads the archive it produced back: it must open as a Word document (content types, relationships, main part, body) and every part it rewrote must parse. If not, the save fails with `save_failed` and no file is produced, so a damaged archive or malformed XML is never handed to the user. (Schema order is checked by the tests and `docx_tool check`, not at save time.) In the browser, files are written through the File System Access API, which replaces the file only when the whole new content has been written; when writing fails, the message explains the likely cause (file open in Word, read-only, permission refused, disk full) and the document stays unsaved, with its draft.

### Undo and redo

`history.rs` keeps a bounded list of **snapshots** (`Document::snapshot`/`restore`). A snapshot holds the main part and the edited secondary parts as XML text (only when edited), the pictures added (shared, not copied), and the relationship and style caches. Restoring re-parses them; restoring the first snapshot gives back the document as opened, so saving writes the original bytes again (`tests/history.rs`, `tests/preservation.rs`).

- The server records a snapshot before each edit batch and drops it if the batch fails. `POST /api/documents/{id}/undo` and `/redo` (body `{base_revision}`) restore one and bump the revision; responses carry `can_undo` and `can_redo`.
- Batches with the same `group` join one step. The editor sends typing syncs with a typing group that changes after any other edit, an undo or redo, typing in another paragraph, or five seconds of continuous typing, so a sentence is undone at once rather than per keystroke.
- At most 200 steps and 96 MiB of snapshots are kept; the oldest go first.
- The editor sends pending typing before undoing, re-renders the page from the server's answer and puts the cursor back where it was.

### Find and replace

`find.js` searches the model's paragraphs (body, table cells, header, footer) using the same text the editor shows, where every object (picture, link, field) is one character that never matches; matches don't cross paragraphs. The query is escaped into a Unicode regular expression (case-insensitive unless *Match case*; *Whole words* uses Unicode letter and digit boundaries). Replacing builds, per paragraph, the same segments the editor would read from the page and turns them into splice pieces: text outside matches keeps its runs, replacement text takes the formatting where the match starts, objects are kept. Replace all is one edit batch, so one undo step. Matches are highlighted with the CSS Custom Highlight API, which doesn't touch the editable DOM.

### Draft recovery

`drafts.js` stores `{id, name, savedAt, data}` (the `.docx` from the download endpoint) in IndexedDB two seconds after a change while the document is unsaved, and deletes it when the document is saved or its changes are discarded. Each open document holds a Web Lock named after its draft, so the startup check (`draftsToOffer`) skips drafts another tab is editing; drafts older than 30 days are removed. When an edit or a reconnection finds that the server no longer has the document (`404`, e.g. after a restart), the editor reopens it from its draft under the same name and says how old the draft is.

### Error handling

- A sync that fails because the server can't be reached (network error or timeout) keeps the edited paragraphs marked dirty and retries every 3 seconds; the text stays on the page and the user is told once. When the connection monitor sees the server again it flushes pending typing and checks that the document still exists.
- Other failed edits reload the document from the server (as before) and say which change could not be applied.
- `nothing_to_undo`/`nothing_to_redo` are shown as information, not errors.

### Testing preservation

`tests/preservation.rs` runs scenarios covering every edit type on the Word-made fixture and on the hard-cases document, and checks for each that only the expected parts were rewritten, that every top-level block the edit didn't touch is byte-identical (longest common subsequence of the body's serialized children), that bookmarks, comment ranges and references, footnote references, complex and simple fields, hyperlinks, content controls, alternate content, extension elements, pictures, section properties, tables and paragraph ids are all still there (in exactly the expected number), that the XML passes `check.rs`, and that saving the result again changes nothing. A final test applies the scenarios one after another and undoes them all, which must give back the original file part for part. All scenario outputs were also opened in Word (`PRESERVATION_OUT` and `scripts/verify-with-word.ps1`), with no repair prompt and with their comments, footnotes, pictures, tables and hyperlinks.

## Rendering and performance

The editor has to stay responsive on long documents. `npm run test:perf` builds a 330-page report (3,000 paragraphs of mixed formatting, 125 tables, 165,000 words) through the API, opens it in headless Edge and times what a user does.

- **Incremental rendering** (`render.js`). After an edit the server returns the whole model, but usually only a block or two changed. Each top-level element remembers the signature of what it was rendered from: its block's JSON, its list marker and, for tables (whose cells' container keys contain it), its index. On re-render, elements whose signature is unchanged are kept and only get their new `data-block`; the page is patched in place rather than rebuilt. A block the user typed into is forgotten on `input` (its DOM no longer matches its signature), so undoing back to an earlier state can't reuse stale content. Loading a document always renders from scratch.
- **Skipping off-screen blocks** (`app.css`). Body blocks use `content-visibility: auto`, so the browser doesn't lay out or paint blocks out of view while typing. It is switched on (class `is-settled`) only after the first full layout, so that `contain-intrinsic-size: auto` has remembered each block's real height and the page estimate stays right. List paragraphs and paragraphs with pictures are excluded, since paint containment would clip their marker or resize handle.
- **Page geometry** is only written to the page's CSS variables when the page setup changed; rewriting them restyled the whole page after every edit.
- **Rolling back** a failed edit batch restores a serialized snapshot instead of keeping a clone of the XML tree. Cloning and dropping the tree cost more than the edit itself on large documents.

Measured on that document (same machine, headless Edge; before → after this work):

| Step | Before | After |
| --- | --- | --- |
| Engine: apply one typing edit | 64 ms | 11 ms |
| Typing sync over HTTP (median) | 147 ms | 97 ms |
| Key press to next frame (median / worst of 20) | 88 / 112 ms | 40 / 88 ms |
| Enter (split a paragraph, until shown) | 1,213 ms | ~540 ms |
| Bold on a word | 1,327 ms | ~300 ms |
| Undo | 1,109 ms | ~550 ms |
| Open (drop the file, until shown) | ~1,060 ms | ~1,350 ms |
| Save (prepare the .docx) | ~350 ms | ~400 ms |

Opening is slightly slower, because blocks are restyled once more when off-screen skipping is switched on. The remaining cost of an edit is mostly building and transferring the whole model (2.9 MB of JSON for this document) and parsing it in the browser; sending only the changed blocks would be the next step.

## Accessibility

- **Structure:** the three toolbars are `role="toolbar"` with labelled groups; the page regions are landmarks (banner, main, contentinfo); the document, header and footer are labelled multi-line text boxes; dialogs are native modal `<dialog>` elements with headings.
- **Keyboard** (`keyboard.js`): each toolbar is one Tab stop with a roving `tabindex`; the arrow keys, Home and End move inside it, and a MutationObserver keeps the stop on an enabled, visible control as controls are enabled and disabled. Alt+F10 goes to the formatting toolbar, Escape back to the page, and F6/Shift+F6 cycle through toolbars, find bar, messages and page. Native dialogs trap the focus and close with Escape; Enter in a field confirms (the browser would use the first button, Cancel); the focus returns to where it was. Tab moves between table cells and changes list levels, as in Word; elsewhere it types a tab, and Shift+Tab leaves the page.
- **States and names:** toggle buttons use `aria-pressed`, Find uses `aria-expanded`/`aria-controls`, buttons list their shortcuts in `aria-keyshortcuts`, and icon buttons have visually hidden labels. The accessibility test reads the browser's accessibility tree and checks that every control has a name.
- **Announcements** (`announce.js`): two visually hidden live regions that are always in the page, one polite (`role="status"`) and one assertive (`role="alert"`), because a region that appears at the moment it changes is often not read. Errors are assertive; opening, creating and saving a document, undo and redo, messages and the draft recovery offer are polite. The find count is its own polite region.
- **Visual:** colours are CSS tokens with a dark set under `prefers-color-scheme: dark` (the sheet stays white, with its own text colours). The accessibility test measures interface text, messages and dialogs at 4.5:1 or better in both themes. Every control has a visible focus outline, and `prefers-reduced-motion` turns off any animation and smooth scrolling.
- **Narrow widths:** under 860 px each toolbar is a single row that scrolls sideways (made a containing block so its visually hidden labels can't widen the page), the margins shrink, and table columns are sized in proportion so tables fit.

## Testing

| Suite | What it covers |
| --- | --- |
| `cargo test` | Engine: parsing, preservation (byte-identical parts and blocks, constructs counted), every edit type on synthetic and Word-made documents, rollback, undo and redo, schema order. Server: routing, security headers and origin checks, every endpoint, embedded assets. |
| `npm test` | The DOM-free frontend modules: content and segments, containers, tables (including Tab order), find, layout, status and page estimates, drafts, files, keyboard navigation, announcements. |
| `npm run test:browser` | `smoke.mjs`: every feature through the real interface in headless Edge/Chrome, on a new document and on the Word-made fixture, with the server's model checked after each step; drafts and offline typing. `accessibility.mjs`: keyboard-only use, announcements, the accessibility tree, contrast in both themes and the phone layout. |
| `npm run test:perf` | The timings above, with generous budgets that catch regressions. |
| Word | `scripts/verify-with-word.ps1` opens edited files in Microsoft Word and checks for repair prompts and content (run during development). |

## Limitations

**Layout and display**
- No pagination: the document is one continuous sheet with dashed lines at each page height. Page breaks are shown as a labelled line, but the text after them is not moved to a new sheet, and the header and footer are shown once, at the top of the sheet and at its end. The page count in the status bar is estimated from the height and the page breaks. Word paginates the saved file normally.
- Only the default header and footer (of the last section) are shown and edited. First-page (`titlePg`) and even-page headers are kept but not shown, and a new header is not added to the first page of a section that has a different first page.
- Floating pictures are shown inline. Tables wider than the text are shown fitted to it, and list numbering inside tables isn't drawn.
- The toolbar shows formatting set directly on the text; formatting that comes only from the paragraph's style (for example a heading's size) shows as the style default.

**Editing**
- Content controls (`w:sdt`), tracked changes, fields, text inside hyperlinks, text boxes, footnotes and comments are preserved and their text is shown, but they are not editable. Nested tables are read-only.
- Splices require every block in the range to be a paragraph, so selections that span a table and the text around it (including *select all* in a document with tables) can't be deleted or typed over. Deleting a paragraph that carries a section break is refused.
- Formatting with nothing selected applies to the word at the cursor; there is no "type in bold from here" mode. Lists restart only where Word's numbering would; there is no "restart numbering" command.
- Tables: no merging or splitting of cells, no resizing columns by dragging, no per-cell borders. Tables whose structure the editor can't follow (rows shifted with `gridBefore`/`gridAfter`, rows in content controls, no grid) refuse row and column edits. Inserting a column keeps percentage and automatic cell widths as they are.
- Pictures are inline only, with no cropping, rotation or text wrapping; they are resized with the mouse or touch, not the keyboard, and can't be pasted from the clipboard. Strict Open XML documents can't get new pictures, headers or footers.
- Page setup applies to the whole document (every section).
- Pasting inserts plain text. Find and replace doesn't search inside hyperlinks, fields, content controls, text boxes, footnotes, comments, nested tables or first-page/even-page headers, and replacement text can't contain line breaks.
- New text copies the formatting of the run it was typed into; a bookmark or comment range right next to edited text can end up just before or after the new text rather than around it.

**Saving and reliability**
- The ZIP container is rebuilt on save: part contents are identical, but compressed sizes and archive metadata may differ from the original file.
- Undo history lives in the server's memory with the open document: it is lost when the document is closed or reopened (also when it is recovered from a draft) and when the server restarts. Undo restores whole document states; the cursor is put back where it was, which may not be where the change happened.
- A draft is written at most every two seconds, so the last moments of typing before a crash may be missing; drafts are only kept in the browser profile that was used, and not in a private window or when site storage is blocked.
- Offline typing is kept on the page only; structural edits (Enter, formatting, tables, …) made while the server is unreachable are refused with a message.
- Each edit returns the whole document model, which is fine for typical documents and acceptable for long ones (see *Rendering and performance*).

**Files**
- Password-protected files and legacy binary `.doc` files can't be opened; UTF-16 encoded parts are rejected (Word always writes UTF-8). Strict OOXML documents are read; their edits are untested against Word.
- Documents are kept in memory while open (at most 32 at a time, 100 MiB per upload, 25 MiB per picture).

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
| `POST /api/documents/{id}/edits` | Body `{base_revision, edits: [...], group?}`: applies the edits atomically → the updated document (with `can_undo`/`can_redo`); batches with the same `group` are undone together. `409 stale_revision` if the revision is outdated, `400 invalid_edit` if an edit is invalid (nothing is changed). |
| `POST /api/documents/{id}/images` | Body: a picture file (at most 25 MiB). Stages it for insertion → `{id, format, width_px, height_px}`; `422 unsupported_image` if it isn't a PNG, JPEG, GIF or BMP. |
| `GET /api/documents/{id}/media/{part}` | An image part the document, a header or a footer references (PNG, JPEG, GIF, BMP, WebP only), for display. |
| `GET /api/documents/{id}/download[?name=…]` | The saved `.docx`; `name` renames the document (Save As). |
| `POST /api/documents/{id}/undo`, `/redo` | Body `{base_revision}`: undo or redo the last edit batch → the updated document. `409 nothing_to_undo`/`nothing_to_redo`, `409 stale_revision`. |
| `DELETE /api/documents/{id}` | Close the document. |

Errors are `{"error": {"code", "message"}}` with an HTTP status: `invalid_document`/`unsupported_document` (422), `document_too_large` (413), `empty_file` (400), `too_many_documents` (409), `not_found` (404), `invalid_edit` (400), `stale_revision`, `nothing_to_undo`, `nothing_to_redo` (409), `unsupported_image` (422), `image_too_large` (413), `save_failed` (500). Parsing and serialization run on Tokio's blocking pool.

The model is JSON: `blocks` (paragraphs with style, alignment, indentation, spacing, numbering and inlines; tables with grid widths, borders, rows and cells with spans, vertical merges, alignment and shading; `other` blocks), `section` (page size, orientation, margins and page number start, in twips), `header`/`footer` (part and blocks), `styles`, `lists`, `headers`/`footers` (all header and footer parts), `stats` and `warnings`. Empty fields are omitted.

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
