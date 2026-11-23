// Browser smoke test: drives the real interface in a headless Chromium-based
// browser (Edge or Chrome) through the DevTools protocol.
//
// Usage:
//   cargo build --release
//   node tests/browser/smoke.mjs            # starts its own server on a free port
//   node tests/browser/smoke.mjs <url>      # or test an already running editor
//
// Set BROWSER to the browser executable if it is not found automatically,
// and SCREENSHOT=<file.png> to save a screenshot of the opened document.
// Native file pickers cannot be automated, so files are opened by dropping
// them onto the workspace, which uses the same code path as "Open".

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const APP_URL_ARG = process.argv[2];
const ROOT = fileURLToPath(new URL("../../", import.meta.url));
const FIXTURE = fileURLToPath(new URL("../fixtures/word-sample.docx", import.meta.url));
const PORT = 9333;

function findBrowser() {
  const candidates = [
    process.env.BROWSER,
    "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/microsoft-edge",
  ].filter(Boolean);
  const found = candidates.find((path) => existsSync(path));
  if (!found) throw new Error("No Chromium-based browser found; set BROWSER to its path.");
  return found;
}

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Starts the built editor on a free port and resolves with its URL. */
function startServer() {
  const exe = process.platform === "win32" ? "docx-editor.exe" : "docx-editor";
  const binary = ["release", "debug"].map((p) => join(ROOT, "target", p, exe)).find((p) => existsSync(p));
  if (!binary) throw new Error("Build the editor first (cargo build --release).");
  const server = spawn(binary, ["--no-open", "--port", "0"], { stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, NO_COLOR: "1" } });
  const url = new Promise((resolve, reject) => {
    let output = "";
    const onData = (chunk) => {
      output += chunk;
      const match = /running at (http:\/\/localhost:\d+)/.exec(output.replace(/\u001b\[[0-9;]*m/g, ""));
      if (match) resolve(match[1]);
    };
    server.stdout.on("data", onData);
    server.stderr.on("data", onData);
    server.on("exit", (code) => reject(new Error(`server exited (${code}): ${output}`)));
  });
  return { server, url };
}

async function connect() {
  for (let attempt = 0; attempt < 50; attempt++) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      const page = targets.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      // Browser still starting.
    }
    await sleep(200);
  }
  throw new Error("Could not connect to the browser's DevTools endpoint.");
}

function cdpClient(url) {
  const socket = new WebSocket(url);
  let nextId = 1;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
    }
  });
  const ready = new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve);
    socket.addEventListener("error", reject);
  });
  return {
    ready,
    send(method, params = {}) {
      const id = nextId++;
      socket.send(JSON.stringify({ id, method, params }));
      return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
    },
    close: () => socket.close(),
  };
}

async function main() {
  const profile = mkdtempSync(join(tmpdir(), "docx-editor-smoke-"));
  const browser = spawn(
    findBrowser(),
    ["--headless=new", "--disable-gpu", "--no-first-run", `--remote-debugging-port=${PORT}`, `--user-data-dir=${profile}`, "about:blank"],
    { stdio: "ignore" },
  );
  const started = APP_URL_ARG ? null : startServer();
  const APP_URL = APP_URL_ARG || (await started.url);
  const failures = [];
  const check = (condition, message) => {
    console.log(`${condition ? "ok  " : "FAIL"} ${message}`);
    if (!condition) failures.push(message);
  };

  try {
    const cdp = cdpClient(await connect());
    await cdp.ready;
    await cdp.send("Runtime.enable");
    const evaluate = async (expression) => {
      const { result, exceptionDetails } = await cdp.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
      if (exceptionDetails) throw new Error(exceptionDetails.exception?.description || exceptionDetails.text);
      return result.value;
    };
    const waitFor = async (expression, timeoutMs = 10000) => {
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        if (await evaluate(expression)) return true;
        await sleep(100);
      }
      return false;
    };
    const text = (id) => evaluate(`document.getElementById(${JSON.stringify(id)}).textContent.trim()`);
    const drop = (base64, name) =>
      evaluate(`(() => {
        const bytes = Uint8Array.from(atob(${JSON.stringify(base64)}), (c) => c.charCodeAt(0));
        const transfer = new DataTransfer();
        transfer.items.add(new File([bytes], ${JSON.stringify(name)}));
        const target = document.getElementById("workspace");
        target.dispatchEvent(new DragEvent("dragover", { dataTransfer: transfer, bubbles: true, cancelable: true }));
        target.dispatchEvent(new DragEvent("drop", { dataTransfer: transfer, bubbles: true, cancelable: true }));
      })()`);

    await cdp.send("Page.navigate", { url: APP_URL });
    check(await waitFor(`document.getElementById("status-connection")?.dataset.state === "connected"`), "connects to the local server");
    check(await evaluate(`document.querySelector('[data-action="save"]').disabled`), "Save is disabled with no document");
    check(await evaluate(`getComputedStyle(document.getElementById("notice")).display === "none"`), "notice bar is hidden at start");

    // Helpers for editing: real key presses and typing through DevTools.
    const press = async (key, code, keyCode) => {
      await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key, code, windowsVirtualKeyCode: keyCode });
      if (key === "Enter") await cdp.send("Input.dispatchKeyEvent", { type: "char", key, code, text: "\r", windowsVirtualKeyCode: keyCode });
      await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key, code, windowsVirtualKeyCode: keyCode });
    };
    const type = (value) => cdp.send("Input.insertText", { text: value });
    // Puts the caret `offset` characters into the text of block `block`.
    const caretAt = (block, offset) =>
      evaluate(`(() => {
        const content = document.getElementById("doc-content");
        content.focus();
        const p = content.querySelector(':scope > [data-block="${block}"]');
        const walker = document.createTreeWalker(p, NodeFilter.SHOW_TEXT);
        let left = ${offset}, node, range = document.createRange();
        range.setStart(p, 0);
        while ((node = walker.nextNode())) {
          if (left <= node.data.length) { range.setStart(node, left); break; }
          left -= node.data.length;
        }
        range.collapse(true);
        getSelection().removeAllRanges();
        getSelection().addRange(range);
      })()`);
    const serverTexts = (id) =>
      evaluate(`fetch("/api/documents/${id}").then((r) => r.json()).then((d) => JSON.stringify(d.model.blocks.filter((b) => b.type === "paragraph").map((b) => (b.inlines || []).flatMap((i) => i.content || []).map((c) => c.text || "").join(""))))`);
    const waitForServer = async (id, expected) => {
      const want = JSON.stringify(expected);
      const deadline = Date.now() + 8000;
      let got;
      while (Date.now() < deadline) {
        got = await serverTexts(id);
        if (got === want) return true;
        await sleep(150);
      }
      console.log(`     server has ${got}`);
      return false;
    };

    // New document: type, Enter, Backspace.
    await evaluate(`document.querySelector('[data-action="new"]').click()`);
    check(await waitFor(`!document.getElementById("doc-content").hidden && document.querySelectorAll("#doc-content .para").length === 1`), "New shows an empty editable page");
    check((await text("document-name")) === "Untitled document.docx", "new document is untitled");
    check((await text("save-state")) === "Not saved yet", "new document is marked not saved");
    check((await evaluate(`document.querySelector("#workspace .page").getAttribute("aria-label")`)).includes("A4, portrait"), "blank document is A4 portrait");
    check(!(await evaluate(`document.querySelector('[data-action="save"]').disabled`)), "Save is enabled");

    await caretAt(0, 0);
    await type("Hello world");
    await press("Enter", "Enter", 13);
    await type("Second paragraph");
    check(await waitForServer(1, ["Hello world", "Second paragraph"]), "typing and Enter reach the document");
    check((await text("save-state")) === "Unsaved changes", "edits mark the document unsaved");
    check((await text("status-words")) === "4 words", `word count follows edits (${await text("status-words")})`);

    await caretAt(1, 0);
    await press("Backspace", "Backspace", 8);
    check(await waitForServer(1, ["Hello worldSecond paragraph"]), "Backspace at a paragraph start joins paragraphs");
    check((await evaluate(`document.querySelectorAll("#doc-content .para").length`)) === 1, "the page shows one paragraph");
    await type(" and more");
    await caretAt(0, 5);
    await press("Delete", "Delete", 46);
    check(await waitForServer(1, ["Helloworld and moreSecond paragraph"]), "after a join the caret is at the join point; Delete works inside a paragraph");

    // Formatting through the toolbar and keyboard.
    const selectText = (block, from, to) =>
      evaluate(`(() => {
        const content = document.getElementById("doc-content");
        content.focus();
        const p = content.querySelector(':scope > [data-block="${block}"]');
        const walker = document.createTreeWalker(p, NodeFilter.SHOW_TEXT);
        const at = (offset) => { let left = offset, node; walker.currentNode = p; while ((node = walker.nextNode())) { if (left <= node.data.length) return [node, left]; left -= node.data.length; } return [p, p.childNodes.length]; };
        const range = document.createRange();
        range.setStart(...at(${from}));
        range.setEnd(...at(${to}));
        getSelection().removeAllRanges();
        getSelection().addRange(range);
        document.dispatchEvent(new Event("selectionchange"));
      })()`);
    const serverModel = (id) => evaluate(`fetch("/api/documents/${id}").then((r) => r.json()).then((d) => d.model)`);
    const waitForModel = async (id, predicate, label) => {
      const deadline = Date.now() + 8000;
      while (Date.now() < deadline) {
        if (predicate(await serverModel(id))) return true;
        await sleep(150);
      }
      console.log(`     ${label}: ${JSON.stringify((await serverModel(id)).blocks[0]).slice(0, 400)}`);
      return false;
    };
    const runs = (m) => m.blocks[0].inlines.filter((i) => i.type === "run");
    const runWith = (m, text) => runs(m).find((r) => r.content.some((c) => c.text === text));

    check(await waitFor(`!document.querySelector('[data-format="bold"]').disabled`), "the formatting toolbar is enabled in a document");
    await selectText(0, 0, 10); // "Helloworld"
    await evaluate(`document.querySelector('[data-format="bold"]').click()`);
    check(await waitForModel(1, (m) => runWith(m, "Helloworld")?.props.bold === true && !runWith(m, " and moreSecond paragraph")?.props.bold, "bold"), "Bold formats only the selection");
    check(await waitFor(`document.querySelector('[data-format="bold"]').getAttribute("aria-pressed") === "true"`), "the Bold button shows the selection is bold");

    await selectText(0, 0, 10);
    await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "i", code: "KeyI", windowsVirtualKeyCode: 73, modifiers: 2 });
    await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: "i", code: "KeyI", windowsVirtualKeyCode: 73, modifiers: 2 });
    check(await waitForModel(1, (m) => runWith(m, "Helloworld")?.props.italic === true && runWith(m, "Helloworld")?.props.bold === true, "italic"), "Ctrl+I adds italic and keeps bold");

    await selectText(0, 0, 10);
    await evaluate(`(() => { const s = document.getElementById("font-size"); s.value = "28"; s.dispatchEvent(new Event("change", { bubbles: true })); })()`);
    check(await waitForModel(1, (m) => runWith(m, "Helloworld")?.props.size === 28, "size"), "the size menu sets 14 pt");

    await caretAt(0, 3);
    await evaluate(`(() => { const s = document.getElementById("style-select"); s.value = "Heading1"; s.dispatchEvent(new Event("change", { bubbles: true })); })()`);
    check(await waitForModel(1, (m) => m.blocks[0].style_id === "Heading1", "style"), "the style menu applies Heading 1");
    check(await waitFor(`!!document.querySelector('#doc-content > [data-block="0"].para-heading-1')`), "the heading is shown with its style");

    await caretAt(0, 3);
    await evaluate(`document.querySelector('[data-format="align"][data-value="center"]').click()`);
    check(await waitForModel(1, (m) => m.blocks[0].alignment === "center", "align"), "Centre aligns the paragraph");

    await caretAt(0, 3);
    await evaluate(`document.querySelector('[data-format="list"][data-value="bullet"]').click()`);
    check(await waitForModel(1, (m) => m.blocks[0].numbering && m.lists.length === 1 && m.lists[0].levels[0].format === "bullet", "bullets"), "Bulleted list creates a list in a new document");
    check(await waitFor(`document.querySelector('#doc-content > [data-block="0"]').dataset.marker === "•"`), "the list marker is shown");
    check(await waitFor(`document.querySelector('[data-format="list"][data-value="bullet"]').getAttribute("aria-pressed") === "true"`), "the list button shows the paragraph is in a list");
    const saved = await evaluate(`fetch("/api/documents/1/download").then((r) => r.arrayBuffer()).then((b) => b.byteLength)`);
    check(saved > 2000, `the formatted document saves (${saved} bytes)`);

    // Insert a table through the Insert table dialog and change it with the table toolbar.
    await caretAt(0, 3);
    await evaluate(`document.querySelector('[data-insert="table"]').click()`);
    check(await waitFor(`document.getElementById("table-dialog").open`), "Insert table opens its dialog");
    await evaluate(`(() => {
      document.getElementById("table-columns").value = "2";
      document.getElementById("table-rows").value = "2";
      document.querySelector('#table-dialog button[value="ok"]').click();
    })()`);
    const tableOf = (m) => m.blocks.find((b) => b.type === "table");
    check(await waitForModel(1, (m) => tableOf(m)?.rows.length === 2 && tableOf(m).rows[0].cells.length === 2, "table"), "a 2 × 2 table is inserted");
    check(await waitFor(`!document.getElementById("table-toolbar").hidden`), "the table toolbar appears with the cursor in the new table");
    await type("Cell one");
    check(
      await waitForModel(1, (m) => tableOf(m)?.rows[0].cells[0].blocks[0].inlines?.[0]?.content?.[0]?.text === "Cell one", "cell"),
      "typing goes into the first cell",
    );
    await evaluate(`document.querySelector('[data-table="row-below"]').click()`);
    check(await waitForModel(1, (m) => tableOf(m)?.rows.length === 3, "row"), "Row below adds a row");
    await evaluate(`document.querySelector('[data-table="column-right"]').click()`);
    check(await waitForModel(1, (m) => tableOf(m)?.grid.length === 3 && tableOf(m).rows.every((r) => r.cells.length === 3), "column"), "Column right adds a column");
    await evaluate(`document.querySelector('[data-table="borders"]').click()`);
    check(await waitForModel(1, (m) => tableOf(m)?.borders === false, "borders"), "Borders switches the table's borders off");
    check(await waitFor(`!!document.querySelector("#doc-content table.table-gridlines")`), "a borderless table shows light gridlines");
    await evaluate(`document.querySelector('[data-table="delete-row"]').click()`);
    check(await waitForModel(1, (m) => tableOf(m)?.rows.length === 2, "delete row"), "Delete row removes the row");

    // Open the Word fixture by dropping it (accept the discard prompt).
    await evaluate(`window.confirm = () => true`);
    await drop(readFileSync(FIXTURE).toString("base64"), "word-sample.docx");
    check(await waitFor(`document.getElementById("document-name").textContent === "word-sample.docx"`), "dropping a .docx opens it");
    check((await text("save-state")) === "Opened", "opened document state");
    check(await evaluate(`getComputedStyle(document.getElementById("notice")).display === "none"`), "no notice after a successful open");
    check((await evaluate(`document.querySelector("#doc-content .para-title")?.textContent`)) === "Quarterly Report", "the title is rendered with its style");
    check((await evaluate(`document.querySelector("#doc-content .para-heading-1")?.textContent`)) === "Summary", "headings are rendered with their style");
    check(await evaluate(`document.querySelectorAll("#doc-content .table-wrap td").length > 0`), "tables are rendered");
    check(await waitFor(`[...document.querySelectorAll("#doc-content .obj-image img")].some((img) => img.complete && img.naturalWidth > 0)`), "images are loaded from the document");
    check(await evaluate(`document.querySelectorAll("#doc-content .obj-link").length > 0`), "hyperlinks are shown");
    check((await text("status-words")).endsWith("words") && (await text("status-words")) !== "0 words", "status bar shows the word count");

    // Replace a word in the title; its style and formatting stay.
    const headingBlock = await evaluate(`Number(document.querySelector("#doc-content .para-title").dataset.block)`);
    await evaluate(`(() => {
      const span = document.querySelector('#doc-content [data-block="${headingBlock}"] [data-kind="run"]');
      const node = span.firstChild;
      const range = document.createRange();
      range.setStart(node, 0);
      range.setEnd(node, "Quarterly".length);
      document.getElementById("doc-content").focus();
      getSelection().removeAllRanges();
      getSelection().addRange(range);
    })()`);
    await type("Annual");
    const before = JSON.parse(await serverTexts(2));
    check(
      await waitFor(`fetch("/api/documents/2").then((r) => r.json()).then((d) => d.model.blocks[${headingBlock}].inlines.map((i) => (i.content || []).map((c) => c.text || "").join("")).join("") === "Annual Report" && d.model.blocks[${headingBlock}].style_id === "Title")`),
      "editing the title keeps its style",
    );
    const after = JSON.parse(await serverTexts(2));
    check(after.length === before.length && after.slice(1).join("|") === before.slice(1).join("|"), "other paragraphs are unchanged");

    // Table cells, the header and the footer are editable.
    const caretIn = (selector, offset) =>
      evaluate(`(() => {
        const p = document.querySelector(${JSON.stringify(selector)});
        p.closest("[contenteditable=true]").focus();
        const walker = document.createTreeWalker(p, NodeFilter.SHOW_TEXT);
        let left = ${offset}, node, range = document.createRange();
        range.setStart(p, 0);
        while ((node = walker.nextNode())) {
          if (left <= node.data.length) { range.setStart(node, left); break; }
          left -= node.data.length;
        }
        range.collapse(true);
        getSelection().removeAllRanges();
        getSelection().addRange(range);
      })()`);
    const tableIndex = await evaluate(`Number(document.querySelector('#doc-content > [data-kind="table"]').dataset.block)`);
    const cellTexts = (row, cell) =>
      evaluate(`fetch("/api/documents/2").then((r) => r.json()).then((d) => JSON.stringify(d.model.blocks[${tableIndex}].rows[${row}].cells[${cell}].blocks.map((b) => (b.inlines || []).flatMap((i) => i.content || []).map((c) => c.text || "").join(""))))`);
    const waitForCell = async (row, cell, expected) => {
      const want = JSON.stringify(expected);
      const deadline = Date.now() + 8000;
      while (Date.now() < deadline) {
        if ((await cellTexts(row, cell)) === want) return true;
        await sleep(150);
      }
      console.log(`     cell has ${await cellTexts(row, cell)}`);
      return false;
    };
    const cellPara = (row, cell, i = 0) => `#doc-content td[data-row="${row}"][data-cell="${cell}"] > [data-block="${i}"]`;
    check(await evaluate(`!!document.querySelector(${JSON.stringify(cellPara(1, 1))})?.closest("[contenteditable=true]")`), "table cells are editable");
    await caretIn(cellPara(1, 1), 2);
    await type("5");
    check(await waitForCell(1, 1, ["105"]), "typing in a table cell changes the cell");
    await press("Enter", "Enter", 13);
    await type("more");
    check(await waitForCell(1, 1, ["105", "more"]), "Enter in a cell adds a paragraph to the cell");
    check(await evaluate(`document.querySelectorAll('#doc-content td[data-row="1"][data-cell="1"] > .para').length === 2`), "the cell shows both paragraphs");

    check((await text("doc-header")).includes("Company Confidential"), "the header is shown");
    check(await evaluate(`document.querySelectorAll("#doc-footer .obj").length > 0`), "the footer's page number field is shown");
    await caretIn("#doc-header .para", "Company Confidential".length);
    await type(" draft");
    check(
      await waitFor(`fetch("/api/documents/2").then((r) => r.json()).then((d) => d.model.header.blocks[0].inlines.map((i) => (i.content || []).map((c) => c.text || "").join("")).join("") === "Company Confidential draft")`),
      "typing in the header changes the header",
    );

    // Pictures: drop one on the page, resize it with its handle, delete it.
    const model2 = () => serverModel(2);
    const imagesIn = (m, block) => (m.blocks[block].inlines || []).flatMap((i) => i.content || []).filter((c) => c.type === "image");
    await caretAt(0, (await evaluate(`document.querySelector('#doc-content > [data-block="0"]').textContent.length`)));
    await evaluate(`(async () => {
      const blob = await (await fetch("/api/documents/2/media/word/media/image1.png")).blob();
      const transfer = new DataTransfer();
      transfer.items.add(new File([blob], "chart.png", { type: "image/png" }));
      const target = document.getElementById("workspace");
      target.dispatchEvent(new DragEvent("drop", { dataTransfer: transfer, bubbles: true, cancelable: true }));
    })()`);
    check(await waitForModel(2, (m) => imagesIn(m, 0).length === 1, "picture"), "a dropped picture is inserted at the cursor");
    const picture = imagesIn(await model2(), 0)[0];
    check(picture.part !== "word/media/image1.png" && picture.description === "chart", `the picture is a new part with a description (${picture.part})`);
    check(await waitFor(`[...document.querySelectorAll('#doc-content > [data-block="0"] .obj-image img')].some((img) => img.complete && img.naturalWidth > 0)`), "the new picture is displayed");

    const box = await evaluate(`JSON.parse(JSON.stringify(document.querySelector('#doc-content > [data-block="0"] .obj-image img').getBoundingClientRect()))`);
    const mouse = (type, x, y) => cdp.send("Input.dispatchMouseEvent", { type, x, y, button: "left", buttons: type === "mouseReleased" ? 0 : 1, clickCount: 1 });
    await mouse("mousePressed", box.x + box.width / 2, box.y + box.height / 2);
    await mouse("mouseReleased", box.x + box.width / 2, box.y + box.height / 2);
    check(await waitFor(`!!document.querySelector("#doc-content .obj-image.is-selected .resize-handle")`), "clicking a picture selects it and shows the resize handle");
    const handle = await evaluate(`JSON.parse(JSON.stringify(document.querySelector(".resize-handle").getBoundingClientRect()))`);
    const hx = handle.x + handle.width / 2;
    const hy = handle.y + handle.height / 2;
    await mouse("mousePressed", hx, hy);
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: hx + 30, y: hy + 10, button: "left", buttons: 1 });
    await cdp.send("Input.dispatchMouseEvent", { type: "mouseMoved", x: hx + 60, y: hy + 20, button: "left", buttons: 1 });
    await mouse("mouseReleased", hx + 60, hy + 20);
    const wider = Math.round(box.width + 60) * 9525;
    check(
      await waitForModel(2, (m) => Math.abs((imagesIn(m, 0)[0]?.width_emu || 0) - wider) <= 9525 * 2, "resize"),
      "dragging the handle resizes the picture in the document",
    );
    const resized = imagesIn(await model2(), 0)[0];
    check(Math.abs(resized.height_emu / resized.width_emu - picture.height_emu / picture.width_emu) < 0.02, "resizing keeps the proportions");

    const box2 = await evaluate(`JSON.parse(JSON.stringify(document.querySelector('#doc-content > [data-block="0"] .obj-image img').getBoundingClientRect()))`);
    await mouse("mousePressed", box2.x + box2.width / 2, box2.y + box2.height / 2);
    await mouse("mouseReleased", box2.x + box2.width / 2, box2.y + box2.height / 2);
    await press("Delete", "Delete", 46);
    check(await waitForModel(2, (m) => imagesIn(m, 0).length === 0, "delete picture"), "Delete removes a selected picture");

    if (process.env.SCREENSHOT) {
      await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
      const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
      writeFileSync(process.env.SCREENSHOT, Buffer.from(data, "base64"));
      console.log(`     screenshot saved to ${process.env.SCREENSHOT}`);
    }

    // An invalid file shows an error and keeps the open document.
    await drop(Buffer.from("not a word file").toString("base64"), "broken.docx");
    check(await waitFor(`!document.getElementById("notice").hidden`), "invalid file shows a notice");
    check((await text("notice-text")).includes("not a valid .docx"), "notice explains the problem");
    check((await text("document-name")) === "word-sample.docx", "the open document stays open");

    // A file that is not a Word document is refused before upload.
    await drop(Buffer.from("hello").toString("base64"), "notes.txt");
    check((await text("notice-text")).includes("is not a Word document"), "non-.docx drops are refused");

    // The open document can be downloaded through the same endpoint Save uses.
    const size = await evaluate(`(async () => {
      const res = await fetch("/api/documents/" + 2 + "/download");
      return res.ok ? (await res.arrayBuffer()).byteLength : -res.status;
    })()`);
    check(size > 20000, `saved document is returned (${size} bytes)`);

    cdp.close();
  } finally {
    const exited = new Promise((resolve) => browser.once("exit", resolve));
    browser.kill();
    started?.server.kill();
    await Promise.race([exited, sleep(5000)]);
    try {
      rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 300 });
    } catch {
      // The browser may still hold files briefly; the OS cleans its temp folder.
    }
  }

  if (failures.length) {
    console.error(`\n${failures.length} check(s) failed.`);
    process.exit(1);
  }
  console.log("\nAll browser checks passed.");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
