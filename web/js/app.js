// Entry point: wires the interface to the local server.

import { closeDocument, downloadDocument, getHealth, newDocument, openDocument } from "./api.js";
import { ensureDocxExtension, outline, pageGeometry, preservedNote, summarize } from "./documents.js";
import { downloadBlob, pickFile, pickSaveLocation, supportsFileSystemAccess, writeToHandle } from "./files.js";
import { connectionLabel, createConnectionMonitor, formatPagePosition, formatWordCount } from "./status.js";

/**
 * The open document, or null.
 * @type {{id: number, name: string, model: object, handle: FileSystemFileHandle|null, savedAt: Date|null, isNew: boolean}|null}
 */
let current = null;
let busy = false;

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
  if (current.savedAt) return `Saved at ${current.savedAt.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}`;
  return current.isNew ? "Not saved yet" : "Opened";
}

function renderDocument() {
  const name = current ? current.name : "Untitled document";
  document.title = `${name} – DOCX Editor`;
  $("document-name").textContent = name;
  $("save-state").textContent = saveStateText();
  $("status-save").textContent = current ? saveStateText() : "No document";
  $("status-page").textContent = formatPagePosition(1, document.querySelectorAll("#workspace .page").length);
  $("status-words").textContent = formatWordCount(current ? current.model.stats.words : 0);

  for (const action of ["save", "save-as"]) {
    document.querySelector(`[data-action="${action}"]`).disabled = busy || !current;
  }
  for (const action of ["new", "open"]) {
    document.querySelector(`[data-action="${action}"]`).disabled = busy;
  }

  $("welcome").hidden = Boolean(current);
  $("doc-summary").hidden = !current;
  applyPageGeometry(current ? current.model.section : null);
  if (current) renderSummary(current);
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
  page.dataset.orientation = geometry.landscape ? "landscape" : "portrait";
  page.setAttribute("aria-label", `Page 1 (${geometry.label})`);
  return geometry;
}

function fillList(list, items, render) {
  list.replaceChildren(...items.map((item) => {
    const li = document.createElement("li");
    render(li, item);
    return li;
  }));
  list.hidden = items.length === 0;
}

function renderSummary(doc) {
  const { model } = doc;
  $("summary-title").textContent = doc.name;
  $("summary-layout").textContent = `Page layout: ${pageGeometry(model.section).label}`;
  fillList($("summary-stats"), summarize(model), (li, text) => (li.textContent = text));

  const note = preservedNote(model);
  $("summary-preserved").textContent = note;
  $("summary-preserved").hidden = !note;
  fillList($("summary-warnings"), model.warnings || [], (li, text) => (li.textContent = text));

  const headings = outline(model);
  fillList($("outline"), headings, (li, heading) => {
    li.textContent = heading.text;
    li.dataset.level = String(heading.level);
  });
  $("outline-title").hidden = headings.length === 0;
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

/** Replaces the open document, closing the previous one on the server. */
function setCurrent(info, { handle = null, isNew = false } = {}) {
  const previous = current;
  current = { id: info.id, name: info.name, model: info.model, handle, savedAt: null, isNew };
  if (previous) closeDocument(previous.id).catch(() => {});
}

function createNew() {
  return run("Creating…", async () => {
    setCurrent(await newDocument(), { isNew: true });
  });
}

function openFile(file, handle = null) {
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

/** Saves to the current file if possible, otherwise asks where to save. */
function save() {
  if (!current) return;
  if (!current.handle && supportsFileSystemAccess()) return saveAs();
  return run("Saving…", async () => {
    const blob = await downloadDocument(current.id);
    if (current.handle) {
      await writeToHandle(current.handle, blob);
    } else {
      downloadBlob(blob, current.name);
    }
    current.savedAt = new Date();
    current.isNew = false;
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

  await run("Saving…", async () => {
    const blob = await downloadDocument(current.id, name);
    if (handle) {
      await writeToHandle(handle, blob);
      current.handle = handle;
    } else {
      downloadBlob(blob, name);
    }
    current.name = name;
    current.savedAt = new Date();
    current.isNew = false;
  });
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

function init() {
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
