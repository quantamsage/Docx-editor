// The editing surface: renders the document, lets the user edit it, and
// keeps the server's document in step.
//
// * Typing and deleting inside one paragraph are left to the browser. The
//   edited paragraph is marked dirty and, shortly after the user pauses,
//   its content is read back and sent as a splice of that paragraph. The
//   surface is not re-rendered, so the caret never jumps; run indices on
//   the page are relabelled to match the server's new model instead.
// * Edits that change paragraph structure (Enter, Backspace/Delete at a
//   paragraph edge, typing or deleting over a selection that spans
//   paragraphs, pasting, Shift+Enter, page breaks, pictures) are
//   intercepted, sent as one splice and the page is re-rendered from the
//   server's answer.
// * Paragraphs live in containers (containers.js): the body, the cells of
//   its tables, and the header and footer, which are separate editing hosts
//   on the page. Every edit names its container; an edit can't span two.
// * All requests run one at a time, in order, and name the document
//   revision they were made against. If one fails, the document is
//   reloaded from the server and the user is told.

import { ApiError, applyEdits, getDocument, mediaUrl } from "../api.js";
import {
  LINE_BREAK,
  formatAt,
  formatRange,
  isNoop,
  listKind,
  normalizeSegments,
  remapAfterSplice,
  replaceRange,
  segmentsLength,
  segmentsToPieces,
  wordAt,
} from "./content.js";
import { containerBlocks, containerField, containerKey, parseContainerKey, parentKey } from "./containers.js";
import { insertTextAtSelection, paragraphLength, parseParagraph, selectionInfo, setCaret, setSelection } from "./dom.js";
import { renderBlocks, renderHeaderFooter } from "./render.js";

/** How long typing must pause before the change is sent. */
export const SYNC_DELAY_MS = 400;

const BLOCKED_INPUT = new Set(["historyUndo", "historyRedo", "insertFromDrop", "deleteByDrag", "insertLink", "insertOrderedList", "insertUnorderedList", "insertHorizontalRule"]);

/** "key#block" for the dirty set. */
const dirtyId = (key, block) => `${key}#${block}`;

/**
 * @param {HTMLElement} root  The element that holds the body's blocks.
 * @param {object} hooks
 * @param {(doc: {id: number, revision: number, model: object}) => void} hooks.onChange  After every applied edit.
 * @param {(message: string) => void} hooks.onError  When an edit could not be applied.
 * @param {(message: string) => void} [hooks.onInfo]  Short informational messages.
 * @param {(name: string) => void} [hooks.onCommand]  A formatting command the browser raised (e.g. "bold").
 * @param {{header?: HTMLElement, footer?: HTMLElement}} [regions]  Editing hosts for the header and footer.
 */
export function createEditor(root, { onChange, onError, onInfo = () => {}, onCommand = () => {} }, regions = {}) {
  /** @type {{id: number, revision: number, model: object}|null} */
  let doc = null;
  const dirty = new Set();
  let timer = null;
  let chain = Promise.resolve();
  let structuralPending = false;
  let composing = false;
  const hosts = [root, regions.header, regions.footer].filter(Boolean);

  /** The editing host holding a container's paragraphs. */
  function hostFor(key) {
    const { part } = parseContainerKey(key);
    if (!part) return root;
    if (doc && doc.model.header && doc.model.header.part === part) return regions.header || null;
    if (doc && doc.model.footer && doc.model.footer.part === part) return regions.footer || null;
    return null;
  }

  /** The element of block `index` of the container with key `key`. */
  function blockEl(index, key = "") {
    const host = hostFor(key);
    if (!host) return null;
    if (!key) return host.querySelector(`:scope > [data-block="${index}"]`);
    for (const el of host.querySelectorAll(`[data-block="${index}"]`)) {
      if ((el.dataset.container || "") === key) return el;
    }
    return null;
  }

  /** The blocks of a container in the current model (empty if gone). */
  const blocksOf = (key) => (doc && containerBlocks(doc.model, key)) || [];

  function render() {
    const ctx = { mediaUrl: (part) => mediaUrl(doc.id, part) };
    renderBlocks(root, doc.model, ctx);
    if (regions.header) renderHeaderFooter(regions.header, doc.model, doc.model.header, ctx);
    if (regions.footer) renderHeaderFooter(regions.footer, doc.model, doc.model.footer, ctx);
    for (const [region, hf] of [[regions.header, doc.model.header], [regions.footer, doc.model.footer]]) {
      if (!region) continue;
      region.contentEditable = hf ? "true" : "false";
      region.dataset.empty = hf ? "false" : "true";
    }
  }

  function load(info) {
    clearTimeout(timer);
    dirty.clear();
    replayQueue.length = 0;
    doc = { id: info.id, revision: info.revision || 0, model: info.model };
    root.contentEditable = "true";
    root.setAttribute("aria-label", "Document text");
    render();
  }

  function unload() {
    clearTimeout(timer);
    dirty.clear();
    replayQueue.length = 0;
    doc = null;
    for (const host of hosts) {
      host.contentEditable = "false";
      host.replaceChildren();
    }
  }

  // ----- Request queue -----

  function enqueue(task) {
    const result = chain.then(task);
    chain = result.catch(() => {});
    return result;
  }

  async function recover(err) {
    const message = err instanceof ApiError || (err && err.message) ? err.message : String(err);
    if (!doc) return;
    const caret = selectionInfo(hosts);
    try {
      const fresh = await getDocument(doc.id);
      load(fresh);
      if (caret && caret.start.editable) placeCaret(caret.start.block, caret.start.offset, caret.start.container);
      onChange(doc);
      onError(`Your last change could not be applied: ${message} The document was reloaded.`);
    } catch (reloadErr) {
      onError(`Your last change could not be applied: ${message} ${reloadErr.message || ""}`.trim());
    }
  }

  /** Sends the dirty paragraphs' content. */
  async function doSync() {
    clearTimeout(timer);
    if (!doc || dirty.size === 0) return;
    const ids = [...dirty];
    dirty.clear();
    const edits = [];
    const sent = [];
    for (const id of ids) {
      const at = id.lastIndexOf("#");
      const key = id.slice(0, at);
      const b = Number(id.slice(at + 1));
      const el = blockEl(b, key);
      const paragraph = blocksOf(key)[b];
      if (!el || !paragraph || paragraph.type !== "paragraph") continue;
      const segments = normalizeSegments(paragraph, parseParagraph(el));
      const pieces = segmentsToPieces(b, paragraph, segments);
      if (isNoop(b, paragraph, pieces)) continue;
      edits.push({ op: "splice", ...containerField(key), first: b, last: b, content: pieces });
      sent.push([key, b, pieces]);
    }
    if (!edits.length) return;
    const result = await applyEdits(doc.id, doc.revision, edits);
    doc.revision = result.revision;
    doc.model = result.model;
    // The page may have changed while the request was out; keep it as it
    // is and point its runs at the new model's indices.
    for (const [key, b, pieces] of sent) relabel(blockEl(b, key), remapAfterSplice(pieces));
    onChange(doc);
  }

  function relabel(el, map) {
    if (!el) return;
    for (const node of el.querySelectorAll("[data-i]")) {
      const old = Number(node.dataset.i);
      if (map.has(old)) node.dataset.i = String(map.get(old));
      else if (node.dataset.kind === "run") node.dataset.i = ""; // its run is gone; text takes a neighbour's format
      else node.dataset.i = "-1";
    }
  }

  function scheduleSync() {
    clearTimeout(timer);
    timer = setTimeout(() => sync().catch(() => {}), SYNC_DELAY_MS);
  }

  /** Sends pending changes now. Resolves when the server has them. */
  function sync() {
    return enqueue(doSync).catch(async (err) => {
      await recover(err);
      throw err;
    });
  }

  // ----- Structural edits -----

  function placeCaret(block, offset, key = "") {
    const el = blockEl(block, key);
    if (!el || el.dataset.kind !== "paragraph") return;
    setCaret(el, Math.min(offset, paragraphLength(el)));
    el.scrollIntoView({ block: "nearest" });
  }

  /** Current segments of paragraph `block` of a container (from the page). */
  function segmentsOf(block, key = "") {
    const paragraph = blocksOf(key)[block];
    return { block, paragraph, segments: normalizeSegments(paragraph, parseParagraph(blockEl(block, key))) };
  }

  /**
   * Runs an edit that changes structure or formatting: pending typing is
   * sent first, then `plan()` builds `{edits, after}` from the current
   * page (or returns null to do nothing), the edits are applied, the page
   * is re-rendered and `after()` restores the caret or selection.
   */
  function runEdit(plan) {
    if (!doc) return Promise.resolve();
    structuralPending = true;
    const task = enqueue(async () => {
      await doSync();
      const built = plan();
      if (!built || !built.edits.length) return;
      const result = await applyEdits(doc.id, doc.revision, built.edits);
      doc.revision = result.revision;
      doc.model = result.model;
      render();
      if (built.after) built.after();
      onChange(doc);
    });
    return task
      .catch((err) => {
        replayQueue.length = 0;
        return recover(err);
      })
      .finally(() => {
        structuralPending = false;
        drainReplayQueue();
      });
  }

  /**
   * Replaces the range from `start` to `end` (positions in one container)
   * with `text`, after inserting `objects` (e.g. a picture or a page break;
   * see content.replaceRange).
   */
  function replace(start, end, text, objects = []) {
    return runEdit(() => {
      const key = start.container || "";
      if ((end.container || "") !== key) {
        onInfo("Selections that cross the edge of a table, header or footer can't be changed.");
        return null;
      }
      const blocks = blocksOf(key);
      for (let b = start.block; b <= end.block; b++) {
        if (blocks[b]?.type !== "paragraph") {
          onInfo("Selections that include tables or protected content can't be changed yet.");
          return null;
        }
      }
      const { splice, caret } = replaceRange(
        { ...segmentsOf(start.block, key), offset: start.offset },
        { ...segmentsOf(end.block, key), offset: end.offset },
        text,
        objects,
      );
      return { edits: [{ op: "splice", ...containerField(key), ...splice }], after: () => placeCaret(caret.block, caret.offset, key) };
    });
  }

  /** The selection, or null (with a message) when it can't be formatted. */
  function editableSelection() {
    const sel = selectionInfo(hosts);
    if (!sel) {
      onInfo("Place the cursor in the document first.");
      return null;
    }
    if (!sel.start.editable || !sel.end.editable) {
      onInfo("Protected content and nested tables can't be formatted.");
      return null;
    }
    if (sel.start.container !== sel.end.container) {
      onInfo("Select text within one table cell, header or footer to format it.");
      return null;
    }
    return sel;
  }

  /**
   * Applies a character format (e.g. {bold: true}) to the selection, or to
   * the word at the cursor when nothing is selected.
   */
  function formatText(format) {
    const sel = editableSelection();
    if (!sel) return Promise.resolve();
    const key = sel.start.container;
    return runEdit(() => {
      let start = { block: sel.start.block, offset: sel.start.offset, container: key };
      let end = { block: sel.end.block, offset: sel.end.offset, container: key };
      if (sel.collapsed) {
        const word = wordAt(segmentsOf(start.block, key).segments, start.offset);
        if (!word) {
          onInfo("Select the text to format.");
          return null;
        }
        start = { ...start, offset: word[0] };
        end = { ...start, offset: word[1] };
      }
      const blocks = blocksOf(key);
      const paragraphs = [];
      for (let b = start.block; b <= end.block; b++) {
        if (blocks[b]?.type === "paragraph") paragraphs.push(segmentsOf(b, key));
      }
      const splices = formatRange(paragraphs, start.offset, end.offset, format);
      const caret = sel.collapsed ? { block: sel.start.block, offset: sel.start.offset } : null;
      return {
        edits: splices.map((s) => ({ op: "splice", ...containerField(key), ...s })),
        after: () => (caret ? placeCaret(caret.block, caret.offset, key) : setSelection(blockEl, start, end)),
      };
    });
  }

  /** Paragraph blocks touched by the selection. */
  function selectedParagraphs(sel) {
    const blocks = blocksOf(sel.start.container);
    const out = [];
    for (let b = sel.start.block; b <= sel.end.block; b++) {
      if (blocks[b]?.type === "paragraph") out.push(b);
    }
    return out;
  }

  /**
   * Applies a paragraph format to the paragraphs in the selection. `format`
   * is a ParagraphFormat, or a function (paragraph, block) → format for
   * per-paragraph changes (e.g. increasing each one's indent).
   */
  function formatParagraphs(format) {
    const sel = editableSelection();
    if (!sel) return Promise.resolve();
    const key = sel.start.container;
    return runEdit(() => {
      const blocks = blocksOf(key);
      const edits = [];
      for (const block of selectedParagraphs(sel)) {
        const f = typeof format === "function" ? format(blocks[block], block) : format;
        if (!f) continue;
        const prev = edits[edits.length - 1];
        // Neighbouring paragraphs with the same change share one edit.
        if (prev && prev.last === block - 1 && JSON.stringify(prev.format) === JSON.stringify(f)) prev.last = block;
        else edits.push({ op: "format_paragraphs", ...containerField(key), first: block, last: block, format: f });
      }
      const start = { block: sel.start.block, offset: sel.start.offset, container: key };
      const end = { block: sel.end.block, offset: sel.end.offset, container: key };
      return { edits, after: () => (sel.collapsed ? placeCaret(start.block, start.offset, key) : setSelection(blockEl, start, end)) };
    });
  }

  /**
   * The table cell at the start of the selection, or null:
   * {container (of the table), table, row, cell}.
   */
  function cellAt(sel) {
    const { part, cell } = parseContainerKey(sel.start.container);
    if (!cell) return null;
    return { container: parentKey(sel.start.container), part, ...cell };
  }

  /**
   * Formatting at the start of the selection, for the toolbar:
   * {run, paragraph, list, paragraphs, container, cell, table}, or null.
   */
  function currentFormat() {
    if (!doc) return null;
    const sel = selectionInfo(hosts);
    if (!sel || !sel.start.editable) return null;
    const key = sel.start.container;
    const { paragraph, segments } = segmentsOf(sel.start.block, key);
    if (!paragraph) return null;
    const offset = sel.collapsed ? sel.start.offset : Math.min(sel.start.offset + 1, segmentsLength(segments));
    const cell = cellAt(sel);
    const table = cell ? blocksOf(cell.container)[cell.table] : null;
    return {
      run: formatAt(paragraph, segments, offset),
      paragraph,
      list: listKind(doc.model, paragraph),
      paragraphs: selectedParagraphs(sel).map((b) => blocksOf(key)[b]),
      container: key,
      cell,
      table,
    };
  }

  function previousParagraph(block, key) {
    return block > 0 && blocksOf(key)[block - 1]?.type === "paragraph" ? block - 1 : null;
  }

  function nextParagraph(block, key) {
    return blocksOf(key)[block + 1]?.type === "paragraph" ? block + 1 : null;
  }

  // ----- Insertions and document-level edits -----

  /** The editable caret position, or null (with a message). */
  function caretPosition(what) {
    const sel = editableSelection();
    if (!sel) return null;
    if (what === "page break" && sel.start.container) {
      onInfo("Page breaks can only be inserted in the main text, not in tables, headers or footers.");
      return null;
    }
    return sel;
  }

  /** Inserts a page break at the cursor (Ctrl+Enter): the text after it starts on a new page. */
  function insertPageBreak() {
    const sel = caretPosition("page break");
    if (!sel) return Promise.resolve();
    return replace(sel.start, sel.end, "\n", [{ type: "page_break" }]);
  }

  /** Inserts a staged picture at the cursor. */
  function insertImage(image) {
    const sel = caretPosition("picture");
    if (!sel) return Promise.resolve();
    return replace(sel.start, sel.end, "", [{ type: "image", ...image }]);
  }

  /** Resizes the picture `inline` of paragraph `block` (sizes in EMUs). */
  function resizeImage(key, block, inline, width_emu, height_emu) {
    return runEdit(() => ({
      edits: [{ op: "resize_image", ...containerField(key), block, inline, width_emu, height_emu }],
      after: () => placeCaret(block, inline + 1, key),
    }));
  }

  /**
   * Inserts a table of `rows` × `columns` at the cursor: in place of an
   * empty paragraph, otherwise after the cursor's paragraph. The cursor
   * moves into its first cell.
   */
  function insertTable(rows, columns) {
    const sel = editableSelection();
    if (!sel) return Promise.resolve();
    if (parseContainerKey(sel.start.container).cell) {
      onInfo("Tables can't be inserted inside a table cell.");
      return Promise.resolve();
    }
    const key = sel.start.container;
    return runEdit(() => {
      const block = sel.start.block;
      const paragraph = blocksOf(key)[block];
      const empty = paragraph && !(paragraph.inlines || []).length;
      const at = empty ? block : block + 1;
      const before = blocksOf(key).length;
      return {
        edits: [{ op: "insert_table", ...containerField(key), at, rows, columns }],
        after: () => {
          const blocks = blocksOf(key);
          const table = blocks.findIndex((b, i) => i >= at && b.type === "table");
          const cell = { ...parseContainerKey(key), cell: { table, row: 0, cell: 0 } };
          if (table >= 0 && blocks.length > before) placeCaret(0, 0, containerKey(cell));
        },
      };
    });
  }

  /**
   * Runs a table command at the cursor's cell. `build(cell, table)` returns
   * the edits and where the cursor goes afterwards ({row, cell}).
   */
  function tableCommand(build) {
    const sel = selectionInfo(hosts);
    const cell = sel && sel.start.editable ? cellAt(sel) : null;
    if (!cell) {
      onInfo("Place the cursor in a table first.");
      return Promise.resolve();
    }
    return runEdit(() => {
      const table = blocksOf(cell.container)[cell.table];
      const built = build(cell, table);
      if (!built) return null;
      return {
        edits: built.edits,
        after: () => {
          if (!built.caret) return;
          const t = blocksOf(cell.container)[cell.table];
          if (!t || t.type !== "table") {
            placeCaret(Math.min(cell.table, blocksOf(cell.container).length - 1), 0, cell.container);
            return;
          }
          const row = Math.max(0, Math.min(built.caret.row, t.rows.length - 1));
          const c = Math.max(0, Math.min(built.caret.cell, t.rows[row].cells.length - 1));
          placeCaret(0, 0, containerKey({ part: cell.part, cell: { table: cell.table, row, cell: c } }));
        },
      };
    });
  }

  /** Applies document-level edits (page setup, headers, footers, page numbers). */
  function applyDocumentEdits(edits, after) {
    return runEdit(() => ({ edits, after }));
  }

  // ----- Events -----

  // Input that arrives while a structural edit is being applied (e.g.
  // typing straight after Enter) is queued and replayed afterwards, so no
  // keystroke is lost.
  const replayQueue = [];

  function onBeforeInput(event) {
    if (!doc) {
      event.preventDefault();
      return;
    }
    const type = event.inputType;
    const data = (event.dataTransfer ? event.dataTransfer.getData("text/plain") : event.data) || "";
    if (structuralPending) {
      if (event.cancelable) event.preventDefault();
      if (!type.startsWith("format") && !BLOCKED_INPUT.has(type)) replayQueue.push({ type, data });
      return;
    }
    if (handleInput(type, data)) event.preventDefault();
  }

  /**
   * Decides what an input does. Returns true when the editor handles it
   * (the browser's default must be prevented), false when the browser may
   * apply it inside the paragraph.
   */
  function handleInput(type, data) {
    if (type.startsWith("format") || BLOCKED_INPUT.has(type)) {
      if (type === "historyUndo" || type === "historyRedo") onInfo("Undo and redo are not available yet.");
      const command = { formatBold: { bold: true }, formatItalic: { italic: true }, formatUnderline: { underline: true }, formatStrikeThrough: { strike: true } }[type];
      if (command) onCommand(Object.keys(command)[0]);
      return true;
    }
    const sel = selectionInfo(hosts);
    if (!sel) return true;
    if (!sel.start.editable || !sel.end.editable) {
      onInfo("Protected content and nested tables can't be edited.");
      return true;
    }
    const key = sel.start.container;
    if (sel.end.container !== key) {
      // Only deletions and typing over such a selection reach here; refuse them.
      onInfo("Selections that cross the edge of a table, header or footer can't be changed.");
      return true;
    }
    const multi = sel.start.block !== sel.end.block;

    switch (type) {
      case "insertParagraph":
        replace(sel.start, sel.end, "\n");
        return true;
      case "insertLineBreak":
        replace(sel.start, sel.end, LINE_BREAK);
        return true;
      case "insertFromPaste":
      case "insertFromPasteAsQuotation":
      case "insertFromYank":
        replace(sel.start, sel.end, data);
        return true;
      case "deleteContentBackward":
      case "deleteWordBackward":
      case "deleteSoftLineBackward":
      case "deleteHardLineBackward":
        if (multi) {
          replace(sel.start, sel.end, "");
          return true;
        }
        if (sel.collapsed && sel.start.offset === 0) {
          const prev = previousParagraph(sel.start.block, key);
          if (prev !== null) replace({ block: prev, container: key, offset: paragraphLength(blockEl(prev, key)) }, sel.start, "");
          return true;
        }
        return false;
      case "deleteContentForward":
      case "deleteWordForward":
      case "deleteSoftLineForward":
      case "deleteHardLineForward":
        if (multi) {
          replace(sel.start, sel.end, "");
          return true;
        }
        if (sel.collapsed && sel.start.offset === paragraphLength(sel.start.el)) {
          const next = nextParagraph(sel.start.block, key);
          if (next !== null) replace(sel.start, { block: next, container: key, offset: 0 }, "");
          return true;
        }
        return false;
      default:
        if (!multi) return false;
        if (type === "insertText" || type === "insertReplacementText") replace(sel.start, sel.end, data);
        else if (type === "deleteByCut" || type === "deleteContent") replace(sel.start, sel.end, "");
        return true;
    }
  }

  /** Applies a queued input that the browser would normally have handled. */
  function applyNatively(type, data) {
    const selection = root.ownerDocument.getSelection();
    if (!selection.rangeCount) return;
    if (type.startsWith("insert")) {
      insertTextAtSelection(root.ownerDocument, data);
    } else if (type.startsWith("delete")) {
      if (selection.isCollapsed && typeof selection.modify === "function") {
        const direction = type.includes("Backward") ? "backward" : "forward";
        const unit = type.includes("Word") ? "word" : type.includes("Line") ? "lineboundary" : "character";
        selection.modify("extend", direction, unit);
      }
      if (!selection.isCollapsed) selection.getRangeAt(0).deleteContents();
    }
    onInput();
  }

  function drainReplayQueue() {
    while (replayQueue.length && !structuralPending && doc) {
      const { type, data } = replayQueue.shift();
      if (!handleInput(type, data)) applyNatively(type, data);
    }
  }

  function markDirtyAtSelection() {
    const sel = selectionInfo(hosts);
    if (!sel || sel.start.container !== sel.end.container) return;
    const key = sel.start.container;
    const blocks = blocksOf(key);
    for (let b = sel.start.block; b <= sel.end.block; b++) {
      if (blocks[b]?.type === "paragraph") dirty.add(dirtyId(key, b));
    }
  }

  function onInput() {
    if (!doc || composing) return;
    markDirtyAtSelection();
    scheduleSync();
  }

  function onKeyDown(event) {
    if (!doc) return;
    const mod = event.ctrlKey || event.metaKey;
    if (mod && !event.altKey && ["z", "y"].includes(event.key.toLowerCase())) {
      event.preventDefault();
      onInfo("Undo and redo are not available yet.");
      return;
    }
    if (mod && !event.altKey && !event.shiftKey && event.key === "Enter") {
      event.preventDefault();
      insertPageBreak();
      return;
    }
    if (event.key === "Tab" && !mod && !event.altKey && !event.shiftKey) {
      const sel = selectionInfo(hosts);
      if (sel && sel.start.editable && sel.start.block === sel.end.block && sel.start.container === sel.end.container) {
        event.preventDefault();
        insertTextAtSelection(root.ownerDocument, "\t");
        onInput();
      }
    }
  }

  function onPaste(event) {
    // Handled through beforeinput (insertFromPaste); stop rich HTML paste
    // in browsers that don't fire it.
    if (!event.defaultPrevented && !("onbeforeinput" in root)) event.preventDefault();
  }

  function onDrop(event) {
    if (event.dataTransfer && !Array.from(event.dataTransfer.types || []).includes("Files")) event.preventDefault();
  }

  for (const host of hosts) {
    host.addEventListener("beforeinput", onBeforeInput);
    host.addEventListener("input", onInput);
    host.addEventListener("keydown", onKeyDown);
    host.addEventListener("paste", onPaste);
    host.addEventListener("drop", onDrop);
    host.addEventListener("compositionstart", () => {
      composing = true;
    });
    host.addEventListener("compositionend", () => {
      composing = false;
      onInput();
    });
  }

  return {
    load,
    unload,
    /** Sends any pending changes; resolves when they are applied. */
    flush: () => (doc ? sync() : Promise.resolve()),
    formatText,
    formatParagraphs,
    currentFormat,
    insertPageBreak,
    insertImage,
    resizeImage,
    insertTable,
    tableCommand,
    applyDocumentEdits,
    placeCaret,
    /** The editing hosts (body, header, footer). */
    hosts,
    /** Whether changes are waiting to be sent. */
    hasPendingChanges: () => dirty.size > 0 || structuralPending || replayQueue.length > 0,
    get document() {
      return doc;
    },
  };
}
