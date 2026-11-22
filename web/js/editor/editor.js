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
//   paragraphs, pasting, Shift+Enter) are intercepted, sent as one splice
//   and the page is re-rendered from the server's answer.
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
import { insertTextAtSelection, paragraphLength, parseParagraph, selectionInfo, setCaret, setSelection } from "./dom.js";
import { renderBlocks } from "./render.js";

/** How long typing must pause before the change is sent. */
export const SYNC_DELAY_MS = 400;

const BLOCKED_INPUT = new Set(["historyUndo", "historyRedo", "insertFromDrop", "deleteByDrag", "insertLink", "insertOrderedList", "insertUnorderedList", "insertHorizontalRule"]);

/**
 * @param {HTMLElement} root  The element that holds the document's blocks.
 * @param {object} hooks
 * @param {(doc: {id: number, revision: number, model: object}) => void} hooks.onChange  After every applied edit.
 * @param {(message: string) => void} hooks.onError  When an edit could not be applied.
 * @param {(message: string) => void} [hooks.onInfo]  Short informational messages.
 * @param {(name: string) => void} [hooks.onCommand]  A formatting command the browser raised (e.g. "bold").
 */
export function createEditor(root, { onChange, onError, onInfo = () => {}, onCommand = () => {} }) {
  /** @type {{id: number, revision: number, model: object}|null} */
  let doc = null;
  const dirty = new Set();
  let timer = null;
  let chain = Promise.resolve();
  let structuralPending = false;
  let composing = false;

  const blockEl = (index) => root.querySelector(`:scope > [data-block="${index}"]`);

  function render() {
    renderBlocks(root, doc.model, { mediaUrl: (part) => mediaUrl(doc.id, part) });
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
    root.contentEditable = "false";
    root.replaceChildren();
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
    const caret = selectionInfo(root);
    try {
      const fresh = await getDocument(doc.id);
      load(fresh);
      if (caret && caret.start.editable) placeCaret(caret.start.block, caret.start.offset);
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
    const blocks = [...dirty].sort((a, b) => a - b);
    dirty.clear();
    const edits = [];
    const sent = new Map();
    for (const b of blocks) {
      const el = blockEl(b);
      const paragraph = doc.model.blocks[b];
      if (!el || !paragraph || paragraph.type !== "paragraph") continue;
      const segments = normalizeSegments(paragraph, parseParagraph(el));
      const pieces = segmentsToPieces(b, paragraph, segments);
      if (isNoop(b, paragraph, pieces)) continue;
      edits.push({ op: "splice", first: b, last: b, content: pieces });
      sent.set(b, pieces);
    }
    if (!edits.length) return;
    const result = await applyEdits(doc.id, doc.revision, edits);
    doc.revision = result.revision;
    doc.model = result.model;
    // The page may have changed while the request was out; keep it as it
    // is and point its runs at the new model's indices.
    for (const [b, pieces] of sent) relabel(blockEl(b), remapAfterSplice(pieces));
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

  function placeCaret(block, offset) {
    const el = blockEl(block);
    if (!el || el.dataset.kind !== "paragraph") return;
    setCaret(el, Math.min(offset, paragraphLength(el)));
    el.scrollIntoView({ block: "nearest" });
  }

  /** Current segments of paragraph `block` (from the page). */
  function segmentsOf(block) {
    const paragraph = doc.model.blocks[block];
    return { block, paragraph, segments: normalizeSegments(paragraph, parseParagraph(blockEl(block))) };
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

  /** Replaces the range from `start` to `end` with `text` (see content.replaceRange). */
  function replace(start, end, text) {
    return runEdit(() => {
      for (let b = start.block; b <= end.block; b++) {
        if (doc.model.blocks[b]?.type !== "paragraph") {
          onInfo("Selections that include tables or protected content can't be changed yet.");
          return null;
        }
      }
      const { splice, caret } = replaceRange({ ...segmentsOf(start.block), offset: start.offset }, { ...segmentsOf(end.block), offset: end.offset }, text);
      return { edits: [{ op: "splice", ...splice }], after: () => placeCaret(caret.block, caret.offset) };
    });
  }

  /** The selection, or null (with a message) when it can't be formatted. */
  function editableSelection() {
    const sel = selectionInfo(root);
    if (!sel) {
      onInfo("Place the cursor in the document first.");
      return null;
    }
    if (!sel.start.editable || !sel.end.editable) {
      onInfo("Tables and protected content can't be formatted yet.");
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
    return runEdit(() => {
      let start = { block: sel.start.block, offset: sel.start.offset };
      let end = { block: sel.end.block, offset: sel.end.offset };
      if (sel.collapsed) {
        const word = wordAt(segmentsOf(start.block).segments, start.offset);
        if (!word) {
          onInfo("Select the text to format.");
          return null;
        }
        start = { block: start.block, offset: word[0] };
        end = { block: start.block, offset: word[1] };
      }
      const paragraphs = [];
      for (let b = start.block; b <= end.block; b++) {
        if (doc.model.blocks[b]?.type === "paragraph") paragraphs.push(segmentsOf(b));
      }
      const splices = formatRange(paragraphs, start.offset, end.offset, format);
      const caret = sel.collapsed ? { block: sel.start.block, offset: sel.start.offset } : null;
      return {
        edits: splices.map((s) => ({ op: "splice", ...s })),
        after: () => (caret ? placeCaret(caret.block, caret.offset) : setSelection(blockEl, start, end)),
      };
    });
  }

  /** Paragraph blocks touched by the selection. */
  function selectedParagraphs(sel) {
    const blocks = [];
    for (let b = sel.start.block; b <= sel.end.block; b++) {
      if (doc.model.blocks[b]?.type === "paragraph") blocks.push(b);
    }
    return blocks;
  }

  /**
   * Applies a paragraph format to the paragraphs in the selection. `format`
   * is a ParagraphFormat, or a function (paragraph, block) → format for
   * per-paragraph changes (e.g. increasing each one's indent).
   */
  function formatParagraphs(format) {
    const sel = editableSelection();
    if (!sel) return Promise.resolve();
    return runEdit(() => {
      const edits = [];
      for (const block of selectedParagraphs(sel)) {
        const f = typeof format === "function" ? format(doc.model.blocks[block], block) : format;
        if (!f) continue;
        const prev = edits[edits.length - 1];
        // Neighbouring paragraphs with the same change share one edit.
        if (prev && prev.last === block - 1 && JSON.stringify(prev.format) === JSON.stringify(f)) prev.last = block;
        else edits.push({ op: "format_paragraphs", first: block, last: block, format: f });
      }
      const start = { block: sel.start.block, offset: sel.start.offset };
      const end = { block: sel.end.block, offset: sel.end.offset };
      return { edits, after: () => (sel.collapsed ? placeCaret(start.block, start.offset) : setSelection(blockEl, start, end)) };
    });
  }

  /**
   * Formatting at the start of the selection, for the toolbar:
   * {run: RunProps, paragraph: Paragraph, list: "bullet"|"number"|null,
   * paragraphs: Paragraph[]}, or null.
   */
  function currentFormat() {
    if (!doc) return null;
    const sel = selectionInfo(root);
    if (!sel || !sel.start.editable) return null;
    const { paragraph, segments } = segmentsOf(sel.start.block);
    const offset = sel.collapsed ? sel.start.offset : Math.min(sel.start.offset + 1, segmentsLength(segments));
    return {
      run: formatAt(paragraph, segments, offset),
      paragraph,
      list: listKind(doc.model, paragraph),
      paragraphs: selectedParagraphs(sel).map((b) => doc.model.blocks[b]),
    };
  }

  function previousParagraph(block) {
    return block > 0 && doc.model.blocks[block - 1]?.type === "paragraph" ? block - 1 : null;
  }

  function nextParagraph(block) {
    return doc.model.blocks[block + 1]?.type === "paragraph" ? block + 1 : null;
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
    const sel = selectionInfo(root);
    if (!sel) return true;
    if (!sel.start.editable || !sel.end.editable) {
      onInfo("Tables and protected content can't be edited yet.");
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
          const prev = previousParagraph(sel.start.block);
          if (prev !== null) replace({ block: prev, offset: paragraphLength(blockEl(prev)) }, sel.start, "");
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
          const next = nextParagraph(sel.start.block);
          if (next !== null) replace(sel.start, { block: next, offset: 0 }, "");
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
    const sel = selectionInfo(root);
    if (!sel) return;
    for (let b = sel.start.block; b <= sel.end.block; b++) {
      if (doc.model.blocks[b]?.type === "paragraph") dirty.add(b);
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
    if (event.key === "Tab" && !mod && !event.altKey && !event.shiftKey) {
      const sel = selectionInfo(root);
      if (sel && sel.start.editable && sel.start.block === sel.end.block) {
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

  root.addEventListener("beforeinput", onBeforeInput);
  root.addEventListener("input", onInput);
  root.addEventListener("keydown", onKeyDown);
  root.addEventListener("paste", onPaste);
  root.addEventListener("drop", onDrop);
  root.addEventListener("compositionstart", () => {
    composing = true;
  });
  root.addEventListener("compositionend", () => {
    composing = false;
    onInput();
  });

  return {
    load,
    unload,
    /** Sends any pending changes; resolves when they are applied. */
    flush: () => (doc ? sync() : Promise.resolve()),
    formatText,
    formatParagraphs,
    currentFormat,
    /** Whether changes are waiting to be sent. */
    hasPendingChanges: () => dirty.size > 0 || structuralPending || replayQueue.length > 0,
    get document() {
      return doc;
    },
  };
}
