// Entry point: wires the interface to the local server.

import { closeDocument, downloadDocument, getHealth, newDocument, openDocument } from "./api.js";
import { ensureDocxExtension, pageGeometry } from "./documents.js";
import { createEditor } from "./editor/editor.js";
import { createInsertTools } from "./editor/insert.js";
import { createToolbar } from "./editor/toolbar.js";
import { downloadBlob, pickFile, pickSaveLocation, supportsFileSystemAccess, writeToHandle } from "./files.js";
import { connectionLabel, createConnectionMonitor, formatPagePosition, formatWordCount } from "./status.js";

/**
 * The open document, or null.
 * @type {{id: number, name: string, model: object, handle: FileSystemFileHandle|null, savedAt: Date|null, isNew: boolean, unsaved: boolean}|null}
 */
let current = null;
let busy = false;
let editor = null;
let toolbar = null;
let insertTools = null;

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

/** Page count and the page the caret is on, from the rendered height. */
function pagePosition() {
  const page = document.querySelector("#workspace .page");
  const pageHeight = parseFloat(getComputedStyle(page).getPropertyValue("--page-height")) || page.clientHeight || 1;
  const total = Math.max(1, Math.ceil((page.scrollHeight - 1) / pageHeight));
  let currentPage = 1;
  const selection = document.getSelection();
  if (current && selection && selection.rangeCount && $("doc-content").contains(selection.anchorNode)) {
    const rect = selection.getRangeAt(0).getBoundingClientRect();
    const top = rect.top || (selection.anchorNode.parentElement || page).getBoundingClientRect().top;
    currentPage = Math.min(total, Math.max(1, Math.floor((top - page.getBoundingClientRect().top) / pageHeight) + 1));
  }
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
  const name = current ? current.name : "Untitled document";
  document.title = `${current && current.unsaved ? "• " : ""}${name} – DOCX Editor`;
  $("document-name").textContent = name;

  for (const action of ["save", "save-as"]) {
    document.querySelector(`[data-action="${action}"]`).disabled = busy || !current;
  }
  for (const action of ["new", "open"]) {
    document.querySelector(`[data-action="${action}"]`).disabled = busy;
  }

  $("welcome").hidden = Boolean(current);
  for (const id of ["doc-content", "doc-header", "doc-footer"]) $(id).hidden = !current;
  document.querySelector("#workspace .page").classList.toggle("has-document", Boolean(current));
  applyPageGeometry(current ? current.model.section : null);
  renderStatus();
  if (toolbar) toolbar.update();
  if (insertTools) insertTools.update();
}

function applyPageGeometry(section) {
  const page = document.querySelector("#workspace .page");
  const geometry = pageGeometry(section);
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

function showNotice(message, kind = "error") {
  $("notice").dataset.kind = kind;
  $("notice-text").textContent = message;
  $("notice").hidden = false;
}

function hideNotice() {
  $("notice").hidden = true;
}

// ----- Actions -----

async function run(label, action) {
  if (busy) return;
  busy = true;
  hideNotice();
  $("save-state").textContent = label;
  renderButtonsOnly();
  try {
    await action();
  } catch (err) {
    showNotice(err && err.message ? err.message : String(err));
  } finally {
    busy = false;
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

/** Replaces the open document, closing the previous one on the server. */
function setCurrent(info, { handle = null, isNew = false } = {}) {
  const previous = current;
  current = { id: info.id, name: info.name, model: info.model, handle, savedAt: null, isNew, unsaved: false };
  editor.load(info);
  if (previous) closeDocument(previous.id).catch(() => {});
}

function createNew() {
  if (!confirmDiscard()) return;
  return run("Creating…", async () => {
    setCurrent(await newDocument(), { isNew: true });
    focusEditor();
  });
}

function openFile(file, handle = null) {
  if (!confirmDiscard()) return;
  return run(`Opening ${file.name}…`, async () => {
    const info = await openDocument(file, file.name);
    setCurrent(info, { handle });
    if (info.model.warnings && info.model.warnings.length) {
      showNotice(info.model.warnings.join(" "), "warning");
    }
  });
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
      await writeToHandle(current.handle, blob);
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
      await writeToHandle(handle, blob);
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
  current.isNew = false;
  current.unsaved = editor.hasPendingChanges();
}

function focusEditor() {
  const content = $("doc-content");
  content.focus();
}

// ----- Wiring -----

function bindToolbar() {
  const actions = { new: createNew, open: chooseAndOpen, save, "save-as": saveAs };
  for (const button of document.querySelectorAll(".toolbar-main [data-action]")) {
    const action = actions[button.dataset.action];
    if (action) button.addEventListener("click", () => action());
  }
  $("notice-close").addEventListener("click", hideNotice);
}

function bindShortcuts() {
  document.addEventListener("keydown", (event) => {
    if (!(event.ctrlKey || event.metaKey) || event.altKey) return;
    const key = event.key.toLowerCase();
    if (key === "o") {
      event.preventDefault();
      chooseAndOpen();
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
    },
    onError(message) {
      showNotice(message);
    },
    onInfo(message) {
      showNotice(message, "warning");
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
    onInfo: (message) => showNotice(message, "warning"),
  });
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
  bindEditor();
  bindToolbar();
  bindShortcuts();
  bindDragAndDrop();
  renderDocument();
  const monitor = createConnectionMonitor({ check: getHealth, onChange: renderConnection });
  monitor.checkNow();
  // Re-check straight away when the window regains focus (e.g. after the
  // server was restarted), instead of waiting for the next poll.
  window.addEventListener("focus", () => monitor.checkNow());
}

init();
