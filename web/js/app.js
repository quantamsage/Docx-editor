// Entry point: wires the interface to the local server.

import { closeDocument, downloadDocument, getDocument, getHealth, newDocument, openDocument } from "./api.js";
import { ensureDocxExtension, pageGeometry } from "./documents.js";
import { DRAFT_DELAY_MS, claimDraft, createDraftStore, draftMessage, draftsToOffer, heldLocks, newDraftId } from "./drafts.js";
import { createEditor } from "./editor/editor.js";
import { createFindBar } from "./editor/findbar.js";
import { enablePictureResizing } from "./editor/images.js";
import { PICTURE_TYPES, createInsertTools } from "./editor/insert.js";
import { createToolbar } from "./editor/toolbar.js";
import { createAnnouncer, openedMessage } from "./announce.js";
import { enableRegionKeys, focusRegion, rovingToolbar } from "./keyboard.js";
import { downloadBlob, pickFile, pickSaveLocation, saveErrorMessage, supportsFileSystemAccess, writeToHandle } from "./files.js";
import { connectionLabel, createConnectionMonitor, estimatePages, formatPagePosition, formatWordCount } from "./status.js";

/**
 * The open document, or null.
 * @type {{id: number, name: string, model: object, handle: FileSystemFileHandle|null, savedAt: Date|null, isNew: boolean, unsaved: boolean}|null}
 */
let current = null;
let busy = false;
let editor = null;
let toolbar = null;
let insertTools = null;
let findBar = null;

// ----- Drafts -----

const drafts = createDraftStore();
/** The open document's draft: {id, lock, timer}. */
let draft = null;

function startDraft(id = newDraftId()) {
  endDraft(false);
  draft = { id, lock: claimDraft(id), timer: null };
}

/** Stops tracking the draft; `remove` deletes it (changes discarded or saved). */
function endDraft(remove) {
  if (!draft) return;
  clearTimeout(draft.timer);
  draft.lock.release();
  if (remove) drafts.remove(draft.id);
  draft = null;
}

function scheduleDraft() {
  if (!draft) return;
  clearTimeout(draft.timer);
  draft.timer = setTimeout(writeDraft, DRAFT_DELAY_MS);
}

/** Stores the document as the server would save it. */
async function writeDraft() {
  if (!draft || !current || !current.unsaved || busy) return;
  const { id } = draft;
  const { id: docId, name } = current;
  try {
    await editor.flush();
    const data = await downloadDocument(docId);
    if (draft && draft.id === id && current && current.id === docId && current.unsaved) {
      await drafts.put({ id, name: current.name || name, savedAt: Date.now(), data });
    }
  } catch {
    // Keep the previous draft; the next change tries again.
  }
}

let offered = [];

/** Offers the next recoverable draft, if any. */
function offerNextDraft() {
  const next = offered[0];
  $("recovery").hidden = !next;
  if (next) {
    $("recovery-text").textContent = draftMessage(next);
    announce(`${draftMessage(next)} Press F6 to reach the Recover button.`);
  }
}

async function findDrafts() {
  const all = await drafts.list();
  const { offer, expired } = draftsToOffer(all, await heldLocks());
  for (const d of expired) drafts.remove(d.id);
  offered = offer.filter((d) => !draft || d.id !== draft.id);
  offerNextDraft();
}

async function recoverDraft() {
  const chosen = offered.shift();
  offerNextDraft();
  if (!chosen || !confirmDiscard()) return;
  const recovered = await run(`Recovering ${chosen.name}…`, async () => {
    const info = await openDocument(chosen.data, chosen.name);
    setCurrent(info, { draftId: chosen.id });
    current.unsaved = true;
  });
  if (!recovered) return;
  focusEditor();
  showNotice(`“${chosen.name}” was recovered from its draft. Save it to keep the changes.`, "warning");
}

function discardDraft() {
  const chosen = offered.shift();
  if (chosen) drafts.remove(chosen.id);
  offerNextDraft();
}

/** The server no longer has the open document: reopen it from its draft. */
async function documentLost() {
  if (!current || busy) return;
  const saved = draft ? await drafts.get(draft.id) : null;
  if (!saved) {
    showNotice(`“${current.name}” is no longer open in the editor server (it was probably restarted), and there is no local draft of it. Open the file again.`);
    return;
  }
  const lostId = current.id;
  await run(`Reopening ${saved.name}…`, async () => {
    const info = await openDocument(saved.data, saved.name);
    if (!current || current.id !== lostId) return;
    const { handle } = current;
    setCurrent(info, { handle, draftId: saved.id, closePrevious: false });
    current.unsaved = true;
    const time = new Date(saved.savedAt).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
    showNotice(`The editor server was restarted, so “${saved.name}” was reopened from its draft saved at ${time}. Changes made in the last seconds before that may be missing.`, "warning");
  });
}

function $(id) {
  return document.getElementById(id);
}

// ----- Rendering -----

function renderConnection(status) {
  $("status-connection").dataset.state = status.state;
  $("status-connection-text").textContent = connectionLabel(status);
}

function saveStateText() {
  if (!current) return "No document open";
  if (current.unsaved) return "Unsaved changes";
  if (current.savedAt) return `Saved at ${current.savedAt.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
  return current.isNew ? "Not saved yet" : "Opened";
}

/** Page count and the page the caret is on, estimated from the rendered height and page breaks. */
function pagePosition() {
  const page = document.querySelector("#workspace .page");
  const style = getComputedStyle(page);
  const pageHeight = parseFloat(style.getPropertyValue("--page-height")) || page.clientHeight || 1;
  const marginTop = parseFloat(style.getPropertyValue("--margin-top")) || 0;
  const pageTop = page.getBoundingClientRect().top;
  const breaks = current ? [...$("doc-content").querySelectorAll(".obj-page_break")].map((b) => b.getBoundingClientRect().bottom - pageTop) : [];
  let caret = null;
  const selection = document.getSelection();
  if (current && selection && selection.rangeCount && $("doc-content").contains(selection.anchorNode)) {
    const rect = selection.getRangeAt(0).getBoundingClientRect();
    caret = (rect.top || (selection.anchorNode.parentElement || page).getBoundingClientRect().top) - pageTop;
  }
  const { current: currentPage, total } = estimatePages(page.scrollHeight, pageHeight, breaks, caret, marginTop);
  return { currentPage, total };
}

function renderStatus() {
  const { currentPage, total } = pagePosition();
  $("status-page").textContent = formatPagePosition(currentPage, total);
  $("status-words").textContent = formatWordCount(current ? current.model.stats.words : 0);
  $("status-save").textContent = current ? saveStateText() : "No document";
  $("save-state").textContent = saveStateText();
}

function renderDocument() {
  // With no document open, the title bar and status bar say only that.
  const name = current ? current.name : "";
  document.title = current ? `${current.unsaved ? "• " : ""}${name} – DOCX Editor` : "DOCX Editor";
  $("document-name").textContent = name;
  $("document-name").hidden = !current;
  $("status-page").hidden = !current;
  $("status-words").hidden = !current;

  for (const action of ["save", "save-as", "find"]) {
    document.querySelector(`[data-action="${action}"]`).disabled = busy || !current;
  }
  for (const action of ["new", "open"]) {
    document.querySelector(`[data-action="${action}"]`).disabled = busy;
  }
  const doc = current && editor ? editor.document : null;
  document.querySelector('[data-action="undo"]').disabled = busy || !doc || !doc.canUndo;
  document.querySelector('[data-action="redo"]').disabled = busy || !doc || !doc.canRedo;

  $("welcome").hidden = Boolean(current);
  for (const id of ["doc-content", "doc-header", "doc-footer"]) $(id).hidden = !current;
  document.querySelector("#workspace .page").classList.toggle("has-document", Boolean(current));
  applyPageGeometry(current ? current.model.section : null);
  renderStatus();
  if (toolbar) toolbar.update();
  if (insertTools) insertTools.update();
}

let appliedGeometry = "";

function applyPageGeometry(section) {
  const page = document.querySelector("#workspace .page");
  const geometry = pageGeometry(section);
  // Changing these inherited properties restyles the whole page, which is
  // slow on long documents: only set them when the page setup changed.
  const key = JSON.stringify(geometry);
  if (key === appliedGeometry) return geometry;
  appliedGeometry = key;
  page.style.setProperty("--page-width", `${geometry.widthPx}px`);
  page.style.setProperty("--page-height", `${geometry.heightPx}px`);
  page.style.setProperty("--margin-top", `${geometry.margins.top}px`);
  page.style.setProperty("--margin-right", `${geometry.margins.right}px`);
  page.style.setProperty("--margin-bottom", `${geometry.margins.bottom}px`);
  page.style.setProperty("--margin-left", `${geometry.margins.left}px`);
  page.style.setProperty("--margin-header", `${Math.min(geometry.margins.header, geometry.margins.top)}px`);
  page.style.setProperty("--margin-footer", `${Math.min(geometry.margins.footer, geometry.margins.bottom)}px`);
  page.dataset.orientation = geometry.landscape ? "landscape" : "portrait";
  page.setAttribute("aria-label", `Document page (${geometry.label})`);
  return geometry;
}

// ----- Notices -----

let announce = () => {};

/**
 * Shows a message above the page and reads it out. `kind` is "error",
 * "warning" (something the user should know about the document) or
 * "info" (the result of an action).
 */
function showNotice(message, kind = "error") {
  $("notice").dataset.kind = kind;
  $("notice-text").textContent = message;
  $("notice").hidden = false;
  announce(message, { urgent: kind === "error" });
}

function hideNotice() {
  $("notice").hidden = true;
}

// ----- Actions -----

/** Runs an action that opens, creates or saves a document; resolves with whether it succeeded. */
async function run(label, action) {
  if (busy) return false;
  busy = true;
  document.querySelector(".app").setAttribute("aria-busy", "true");
  hideNotice();
  $("save-state").textContent = label;
  renderButtonsOnly();
  try {
    await action();
    return true;
  } catch (err) {
    showNotice(err && err.message ? err.message : String(err));
    return false;
  } finally {
    busy = false;
    document.querySelector(".app").removeAttribute("aria-busy");
    renderDocument();
  }
}

function renderButtonsOnly() {
  for (const button of document.querySelectorAll(".toolbar-main [data-action]")) {
    if (["new", "open", "save", "save-as"].includes(button.dataset.action)) button.disabled = true;
  }
}

/** Asks before discarding unsaved changes. */
function confirmDiscard() {
  if (!current || (!current.unsaved && !editor.hasPendingChanges())) return true;
  return window.confirm(`“${current.name}” has unsaved changes. Discard them?`);
}

/**
 * Replaces the open document, closing the previous one on the server (its
 * unsaved changes were discarded, so its draft is deleted). `draftId`
 * continues an existing draft (recovery).
 */
function setCurrent(info, { handle = null, isNew = false, draftId = null, closePrevious = true } = {}) {
  const previous = current;
  if (!draftId || !draft || draft.id !== draftId) {
    endDraft(true);
    startDraft(draftId || undefined);
  }
  current = { id: info.id, name: info.name, model: info.model, handle, savedAt: null, isNew, unsaved: false };
  editor.load(info);
  if (previous && closePrevious) closeDocument(previous.id).catch(() => {});
}

async function createNew() {
  if (!confirmDiscard()) return;
  // The page is shown when run() ends, so the cursor is put in it after.
  if (await run("Creating…", async () => setCurrent(await newDocument(), { isNew: true }))) {
    focusEditor();
    announce(openedMessage(current.name, 0, true));
  }
}

async function openFile(file, handle = null) {
  if (!confirmDiscard()) return;
  let warnings = [];
  const opened = await run(`Opening ${file.name}…`, async () => {
    const info = await openDocument(file, file.name);
    setCurrent(info, { handle });
    warnings = info.model.warnings || [];
  });
  if (!opened) return;
  focusEditor();
  if (warnings.length) showNotice(warnings.join(" "), "warning");
  else announce(openedMessage(current.name, current.model.stats.words, false));
}

async function chooseAndOpen() {
  if (busy) return;
  let picked;
  try {
    picked = await pickFile();
  } catch (err) {
    showNotice(err.message);
    return;
  }
  if (picked) await openFile(picked.file, picked.handle);
}

/** Sends pending edits; returns false if they could not be applied. */
async function flushEdits() {
  try {
    await editor.flush();
    return true;
  } catch {
    return false; // the editor reloaded the document and showed why
  }
}

/** Saves to the current file if possible, otherwise asks where to save. */
async function save() {
  if (!current) return;
  if (!current.handle && supportsFileSystemAccess()) return saveAs();
  if (!(await flushEdits())) return;
  return run("Saving…", async () => {
    const blob = await downloadDocument(current.id);
    if (current.handle) {
      try {
        await writeToHandle(current.handle, blob);
      } catch (err) {
        throw new Error(saveErrorMessage(err, current.name));
      }
    } else {
      downloadBlob(blob, current.name);
    }
    markSaved();
  });
}

async function saveAs() {
  if (!current || busy) return;
  let handle = null;
  let name;
  if (supportsFileSystemAccess()) {
    try {
      handle = await pickSaveLocation(current.name);
    } catch (err) {
      showNotice(err.message);
      return;
    }
    if (!handle) return;
    name = handle.name;
  } else {
    const answer = window.prompt("Save as", current.name);
    if (answer === null) return;
    name = ensureDocxExtension(answer);
  }
  if (!(await flushEdits())) return;

  await run("Saving…", async () => {
    const blob = await downloadDocument(current.id, name);
    if (handle) {
      try {
        await writeToHandle(handle, blob);
      } catch (err) {
        throw new Error(saveErrorMessage(err, name));
      }
      current.handle = handle;
    } else {
      downloadBlob(blob, name);
    }
    current.name = name;
    markSaved();
  });
}

function markSaved() {
  current.savedAt = new Date();
  announce(`Saved ${current.name}.`);
  current.isNew = false;
  current.unsaved = editor.hasPendingChanges();
  // The file on disk now has the changes: the draft is no longer needed.
  if (draft && !current.unsaved) {
    clearTimeout(draft.timer);
    drafts.remove(draft.id);
  }
}

function focusEditor() {
  const content = $("doc-content");
  content.focus();
}

// ----- Wiring -----

function bindToolbar() {
  const actions = {
    new: createNew,
    open: chooseAndOpen,
    save,
    "save-as": saveAs,
    undo: () => editor.undo(),
    redo: () => editor.redo(),
    find: () => findBar.open(true),
  };
  for (const button of document.querySelectorAll(".toolbar-main [data-action]")) {
    const action = actions[button.dataset.action];
    if (action) button.addEventListener("click", () => action());
  }
  $("notice-close").addEventListener("click", hideNotice);
  $("recovery-open").addEventListener("click", () => recoverDraft());
  $("recovery-discard").addEventListener("click", discardDraft);
}

/** Keyboard access to the toolbars and the regions of the window (keyboard.js). */
function bindKeyboardNavigation() {
  const toolbars = [...document.querySelectorAll('[role="toolbar"]')];
  for (const bar of toolbars) rovingToolbar(bar);
  const documentRegion = () => (current ? editor.hosts.find((h) => h.contains(document.getSelection()?.anchorNode)) || $("doc-content") : $("welcome"));
  enableRegionKeys(
    () => [...toolbars, $("find-bar"), $("recovery"), $("notice"), documentRegion()],
    () => {
      const format = $("format-toolbar");
      focusRegion(format.querySelector("[tabindex='0']:not(:disabled)") ? format : toolbars[0]);
    },
  );
  // Escape in a toolbar goes back to the document.
  for (const bar of toolbars) {
    bar.addEventListener("keydown", (event) => {
      if (event.key === "Escape" && current && !event.defaultPrevented) {
        event.preventDefault();
        documentRegion().focus();
      }
    });
  }
}

function bindShortcuts() {
  document.addEventListener("keydown", (event) => {
    if (event.key === "Escape" && findBar.isOpen && !event.defaultPrevented) {
      event.preventDefault();
      findBar.close();
      return;
    }
    if (!(event.ctrlKey || event.metaKey) || event.altKey) return;
    const key = event.key.toLowerCase();
    if (key === "o") {
      event.preventDefault();
      chooseAndOpen();
    } else if ((key === "f" || key === "h") && current) {
      event.preventDefault();
      findBar.open(key === "h");
    } else if (key === "s") {
      event.preventDefault();
      if (event.shiftKey) saveAs();
      else save();
    }
  });
}

function bindDragAndDrop() {
  const workspace = $("workspace");
  const hasFiles = (event) => Array.from(event.dataTransfer?.types || []).includes("Files");
  workspace.addEventListener("dragover", (event) => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    event.dataTransfer.dropEffect = "copy";
    workspace.classList.add("is-drop-target");
  });
  workspace.addEventListener("dragleave", () => workspace.classList.remove("is-drop-target"));
  workspace.addEventListener("drop", (event) => {
    if (!hasFiles(event)) return;
    event.preventDefault();
    workspace.classList.remove("is-drop-target");
    const file = event.dataTransfer.files[0];
    if (!file) return;
    // A picture dropped on the page is inserted where it was dropped.
    if (current && (PICTURE_TYPES.includes(file.type) || /\.(png|jpe?g|gif|bmp)$/i.test(file.name))) {
      const caret = document.caretRangeFromPoint ? document.caretRangeFromPoint(event.clientX, event.clientY) : null;
      if (caret && editor.hosts.some((h) => h.contains(caret.startContainer))) {
        getSelection().removeAllRanges();
        getSelection().addRange(caret);
      }
      insertTools.insertPicture(file);
      return;
    }
    if (!/\.(docx|docm|dotx|dotm)$/i.test(file.name)) {
      showNotice(`"${file.name}" is not a Word document (.docx).`);
      return;
    }
    openFile(file);
  });
}

function bindEditor() {
  editor = createEditor($("doc-content"), {
    onChange(doc) {
      if (!current || current.id !== doc.id) return;
      current.model = doc.model;
      current.unsaved = true;
      renderDocument();
      findBar.refresh();
      scheduleDraft();
    },
    onError(message) {
      showNotice(message);
    },
    onInfo(message) {
      showNotice(message, "info");
    },
    onAnnounce(message) {
      announce(message);
    },
    onLost() {
      documentLost();
    },
    onCommand(name) {
      toolbar.command(name);
    },
  }, { header: $("doc-header"), footer: $("doc-footer") });
  toolbar = createToolbar($("format-toolbar"), editor);
  insertTools = createInsertTools({
    editor,
    insertBar: $("insert-tools"),
    tableBar: $("table-toolbar"),
    onInfo: (message) => showNotice(message, "info"),
    onError: (message) => showNotice(message),
  });
  enablePictureResizing(editor);
  findBar = createFindBar({ editor, bar: $("find-bar"), onInfo: (message) => showNotice(message, "info") });
  for (const host of editor.hosts) {
    host.addEventListener("keydown", (event) => toolbar.handleKey(event));
    // Typing changes the page count and the caret's page.
    host.addEventListener("input", () => {
      if (current && !current.unsaved) {
        current.unsaved = true;
        renderDocument();
      } else {
        renderStatus();
      }
    });
  }
  let pending = 0;
  document.addEventListener("selectionchange", () => {
    if (!current) return;
    renderStatus();
    cancelAnimationFrame(pending);
    pending = requestAnimationFrame(() => {
      toolbar.update();
      insertTools.update();
    });
  });
  window.addEventListener("beforeunload", (event) => {
    if (current && (current.unsaved || editor.hasPendingChanges())) {
      event.preventDefault();
      event.returnValue = "";
    }
  });
}

function init() {
  announce = createAnnouncer($("announce-polite"), $("announce-urgent"));
  bindEditor();
  bindToolbar();
  bindShortcuts();
  bindKeyboardNavigation();
  bindDragAndDrop();
  renderDocument();
  let wasOffline = false;
  const monitor = createConnectionMonitor({
    check: getHealth,
    onChange(status) {
      renderConnection(status);
      // Back after an outage: check that the server still has the document.
      if (status.state === "connected" && wasOffline && current) {
        getDocument(current.id).catch((err) => {
          if (err && err.code === "not_found") documentLost();
        });
        editor.flush().catch(() => {});
      }
      wasOffline = status.state === "offline";
    },
  });
  findDrafts();
  monitor.checkNow();
  // Re-check straight away when the window regains focus (e.g. after the
  // server was restarted), instead of waiting for the next poll.
  window.addEventListener("focus", () => monitor.checkNow());
}

init();
