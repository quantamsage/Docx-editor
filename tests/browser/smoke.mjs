// Browser smoke test: drives the real interface in a headless Chromium-based
// browser (Edge or Chrome) through the DevTools protocol.
//
// Usage:
//   cargo build --release
//   node tests/browser/smoke.mjs            # starts its own server on a free port
//   node tests/browser/smoke.mjs <url>      # or test an already running editor
//
// Set BROWSER to the browser executable if it is not found automatically,
// SCREENSHOT=<file.png> to save a screenshot of the opened document,
// SCREENSHOT_DIR=<folder> to save screenshots of the main screens at desktop
// and phone widths, and SAVE_DIR=<folder> to save the edited documents (for
// checking them in Word).
// Native file pickers cannot be automated, so files are opened by dropping
// them onto the workspace, which uses the same code path as "Open".

import { readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { checker, launch, sleep } from "./harness.mjs";

const APP_URL_ARG = process.argv[2];
const FIXTURE = fileURLToPath(new URL("../fixtures/word-sample.docx", import.meta.url));
const PORT = 9333;

async function main() {
  const session = await launch({ port: PORT, appUrl: APP_URL_ARG, name: "smoke" });
  const { cdp, evaluate, waitFor, drop, url: APP_URL } = session;
  const { check, failures } = checker();

  try {
    const text = (id) => evaluate(`document.getElementById(${JSON.stringify(id)}).textContent.trim()`);

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
        document.dispatchEvent(new Event("selectionchange"));
      })()`);
    const cellPara0 = '#doc-content td[data-row="0"][data-cell="0"] > [data-block="0"]';

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

    // Add a header and type in it; number the pages in the footer.
    check(await evaluate(`document.getElementById("doc-header").dataset.empty === "true"`), "a new document has no header");
    await caretAt(0, 0);
    await evaluate(`document.querySelector('[data-insert="header"]').click()`);
    check(await waitForModel(1, (m) => m.header?.part === "word/header1.xml", "header"), "Header adds a header to the document");
    check(await waitFor(`document.getElementById("doc-header").isContentEditable && document.getSelection().anchorNode && document.getElementById("doc-header").contains(document.getSelection().anchorNode)`), "the cursor moves into the new header");
    await type("Project notes");
    check(
      await waitForModel(1, (m) => m.header?.blocks[0].inlines?.[0]?.content?.[0]?.text === "Project notes", "header text"),
      "typing in the new header is saved in the header part",
    );
    await caretAt(0, 0);
    await evaluate(`document.querySelector('[data-insert="page-numbers"]').click()`);
    check(await waitFor(`document.getElementById("page-numbers-dialog").open`), "Page numbers opens its dialog");
    await evaluate(`(() => {
      document.getElementById("page-numbers-alignment").value = "right";
      document.getElementById("page-numbers-total").checked = true;
      document.querySelector('#page-numbers-dialog button[value="ok"]').click();
    })()`);
    const footerFields = (m) => (m.footer?.blocks || []).flatMap((b) => b.inlines || []).filter((i) => i.field).map((i) => i.field);
    check(await waitForModel(1, (m) => footerFields(m).join() === "PAGE,NUMPAGES" && m.footer.blocks[0].alignment === "right", "page numbers"), "page numbers with the page count are added to the footer");
    check(await waitFor(`document.querySelector("#doc-footer .obj-field")?.textContent === "1"`), "the footer shows the page number");

    // Page setup: Letter, landscape, narrow margins.
    await caretAt(0, 0);
    await evaluate(`document.querySelector('[data-insert="page-setup"]').click()`);
    check(await waitFor(`document.getElementById("page-setup-dialog").open && document.getElementById("page-paper").value === "a4"`), "Page setup shows the current paper size");
    await evaluate(`(() => {
      document.getElementById("page-paper").value = "letter";
      document.getElementById("page-landscape").checked = true;
      const preset = document.getElementById("page-margin-preset");
      preset.value = "narrow";
      preset.dispatchEvent(new Event("change"));
      document.querySelector('#page-setup-dialog button[value="ok"]').click();
    })()`);
    check(
      await waitForModel(1, (m) => m.section.page_width === 15840 && m.section.page_height === 12240 && m.section.landscape && m.section.margin_left === 720, "page setup"),
      "Page setup changes paper size, orientation and margins",
    );
    check(await waitFor(`document.querySelector("#workspace .page").getAttribute("aria-label").includes("Letter, landscape")`), "the page is shown in its new size");

    // Page breaks: the toolbar button and Ctrl+Enter.
    const pageBreaks = (m) => m.blocks.filter((b) => b.type === "paragraph" && (b.inlines || []).some((i) => (i.content || []).some((c) => c.type === "page_break"))).length;
    await caretAt(0, 5);
    await evaluate(`document.querySelector('[data-insert="page-break"]').click()`);
    check(await waitForModel(1, (m) => pageBreaks(m) === 1, "page break"), "Page break inserts a page break");
    await type("X");
    await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, modifiers: 2 });
    await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Enter", code: "Enter", windowsVirtualKeyCode: 13, modifiers: 2 });
    check(await waitForModel(1, (m) => pageBreaks(m) === 2, "ctrl+enter"), "Ctrl+Enter inserts a page break");
    check(await waitFor(`document.querySelectorAll("#doc-content .obj-page_break").length === 2`), "page breaks are shown");

    // Undo and redo: keyboard shortcuts and toolbar buttons.
    const ctrl = async (key, code, keyCode, shift = false) => {
      const modifiers = shift ? 10 : 2;
      await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key, code, windowsVirtualKeyCode: keyCode, modifiers });
      await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key, code, windowsVirtualKeyCode: keyCode, modifiers });
    };
    check(await waitFor(`!document.querySelector('[data-action="undo"]').disabled`), "Undo is enabled after edits");
    await ctrl("z", "KeyZ", 90);
    check(await waitForModel(1, (m) => pageBreaks(m) === 1, "undo"), "Ctrl+Z undoes the last page break");
    check(await waitFor(`!document.querySelector('[data-action="redo"]').disabled`), "Redo is enabled after undoing");
    await ctrl("y", "KeyY", 89);
    check(await waitForModel(1, (m) => pageBreaks(m) === 2, "redo"), "Ctrl+Y redoes it");
    await evaluate(`document.querySelector('[data-action="undo"]').click()`);
    check(await waitForModel(1, (m) => pageBreaks(m) === 1, "undo button"), "the Undo button undoes");
    await evaluate(`document.querySelector('[data-action="redo"]').click()`);
    check(await waitForModel(1, (m) => pageBreaks(m) === 2, "redo button"), "the Redo button redoes");
    const lastBlock = await evaluate(`document.querySelectorAll("#doc-content > [data-block]").length - 1`);
    const textOf = (m, b) => (m.blocks[b].inlines || []).flatMap((i) => i.content || []).map((c) => c.text || "").join("");
    const original = textOf(await serverModel(1), lastBlock);
    await caretAt(lastBlock, 0);
    await type("Typed then undone ");
    check(await waitForModel(1, (m) => textOf(m, lastBlock) === `Typed then undone ${original}`, "typed"), "typing reaches the document");
    await ctrl("z", "KeyZ", 90);
    check(await waitForModel(1, (m) => textOf(m, lastBlock) === original, "undo typing"), "Ctrl+Z undoes the typing in one step");
    check(await waitFor(`document.querySelector('#doc-content > [data-block="${lastBlock}"]').textContent === ${JSON.stringify(original)}`), "the page shows the undone text");
    await ctrl("z", "KeyZ", 90, true);
    check(await waitForModel(1, (m) => textOf(m, lastBlock) === `Typed then undone ${original}`, "redo typing"), "Ctrl+Shift+Z redoes it");

    const shot = async (name, width = 1280, height = 900) => {
      if (!process.env.SCREENSHOT_DIR) return;
      await cdp.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: width < 600 });
      await sleep(400);
      const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
      writeFileSync(join(process.env.SCREENSHOT_DIR, `${name}.png`), Buffer.from(data, "base64"));
      await cdp.send("Emulation.clearDeviceMetricsOverride");
    };
    const saveDocument = async (id, name) => {
      if (!process.env.SAVE_DIR) return;
      const base64 = await evaluate(`fetch("/api/documents/${id}/download").then((r) => r.arrayBuffer()).then((b) => { let s = ""; new Uint8Array(b).forEach((c) => (s += String.fromCharCode(c))); return btoa(s); })`);
      writeFileSync(join(process.env.SAVE_DIR, name), Buffer.from(base64, "base64"));
    };
    await caretIn(cellPara0, 0);
    await shot("new-document-desktop");
    await shot("new-document-narrow", 420, 860);
    await evaluate(`document.querySelector('[data-insert="page-setup"]').click()`);
    await waitFor(`document.getElementById("page-setup-dialog").open`);
    await shot("page-setup-dialog");
    await shot("page-setup-dialog-narrow", 420, 860);
    await evaluate(`document.querySelector('#page-setup-dialog button[value="cancel"]').click()`);
    await saveDocument(1, "browser-new.docx");

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

    // Find and replace.
    const bodyText = () => evaluate(`fetch("/api/documents/2").then((r) => r.json()).then((d) => JSON.stringify(d.model.blocks))`);
    const count = async (word) => (((await bodyText()).match(/"text":"[^"]*"/g) || []).join(" ").match(new RegExp(word, "gi")) || []).length;
    const paragraphs = await count("paragraph");
    const waitForCount = async (word, n) => {
      const deadline = Date.now() + 8000;
      while (Date.now() < deadline) {
        if ((await count(word)) === n) return true;
        await sleep(150);
      }
      console.log(`     "${word}" appears ${await count(word)} times`);
      return false;
    };
    await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "h", code: "KeyH", windowsVirtualKeyCode: 72, modifiers: 2 });
    await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: "h", code: "KeyH", windowsVirtualKeyCode: 72, modifiers: 2 });
    check(await waitFor(`!document.getElementById("find-bar").hidden && !document.getElementById("replace-row").hidden`), "Ctrl+H opens find and replace");
    await evaluate(`(() => { const i = document.getElementById("find-input"); i.focus(); i.value = "PARAGRAPH"; i.dispatchEvent(new Event("input", { bubbles: true })); })()`);
    check(await waitFor(`document.getElementById("find-count").textContent === "1 of ${paragraphs}"`), `matches are counted (${paragraphs})`);
    check(await evaluate(`!window.CSS?.highlights || CSS.highlights.get("find-matches")?.size === ${paragraphs}`), "matches are highlighted");
    await press("Enter", "Enter", 13);
    check(await waitFor(`document.getElementById("find-count").textContent === "2 of ${paragraphs}"`), "Enter moves to the next match");
    await shot("find-bar");
    await shot("find-bar-narrow", 420, 860);
    await evaluate(`(() => { document.getElementById("find-word").checked = true; document.getElementById("find-word").dispatchEvent(new Event("change")); })()`);
    await evaluate(`(() => { const r = document.getElementById("replace-input"); r.value = "section"; document.querySelector('[data-find="replace-all"]').click(); })()`);
    check(await waitForCount("paragraph", 0), "Replace all replaces every match");
    check((await count("section")) >= paragraphs, "the replacement text is in the document");
    check(await waitFor(`document.getElementById("find-count").textContent === "No results"`), "no matches are left");
    await evaluate(`document.querySelector('[data-action="undo"]').click()`);
    check(await waitForCount("paragraph", paragraphs), "one Undo restores every replacement");
    await evaluate(`(() => { const i = document.getElementById("find-input"); i.value = "paragraph"; i.dispatchEvent(new Event("input", { bubbles: true })); })()`);
    check(await waitFor(`document.getElementById("find-count").textContent === "1 of ${paragraphs}"`), "the matches are found again after undoing");
    await evaluate(`document.querySelector('[data-find="replace"]').click()`);
    check(await waitForCount("paragraph", paragraphs - 1), "Replace replaces one match");
    await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
    await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key: "Escape", code: "Escape", windowsVirtualKeyCode: 27 });
    check(await waitFor(`document.getElementById("find-bar").hidden`), "Escape closes the find bar");

    await caretIn(cellPara(1, 1), 0);
    await shot("word-document-table-desktop");
    await shot("word-document-table-narrow", 420, 860);
    await saveDocument(2, "browser-word-sample.docx");

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

    // Draft recovery: an edited document is kept as a local draft.
    const draftCount = () =>
      evaluate(`new Promise((resolve) => {
        const open = indexedDB.open("docx-editor-drafts", 1);
        open.onupgradeneeded = () => open.result.createObjectStore("drafts", { keyPath: "id" });
        open.onsuccess = () => {
          const all = open.result.transaction("drafts").objectStore("drafts").getAll();
          all.onsuccess = () => { resolve(all.result.map((d) => d.name + ":" + d.data.size).join(",")); open.result.close(); };
        };
        open.onerror = () => resolve("error");
      })`);
    const titleBlock = await evaluate(`Number(document.querySelector("#doc-content .para-title").dataset.block)`);
    await caretAt(titleBlock, 0);
    await type("Draft check ");
    check(await waitFor(`fetch("/api/documents/2").then((r) => r.json()).then((d) => JSON.stringify(d.model.blocks[${titleBlock}]).includes("Draft check"))`), "the edit reaches the server");
    const drafted = await (async () => {
      const deadline = Date.now() + 8000;
      while (Date.now() < deadline) {
        if ((await draftCount()).startsWith("word-sample.docx:")) return true;
        await sleep(250);
      }
      return false;
    })();
    check(drafted, `a draft of the edited document is stored (${await draftCount()})`);

    // The server loses the document (as after a restart): it is reopened from the draft.
    await evaluate(`fetch("/api/documents/2", { method: "DELETE" })`);
    await caretAt(titleBlock, 0);
    await type("x");
    check(await waitFor(`document.getElementById("notice-text").textContent.includes("reopened from its draft")`), "a document the server lost is reopened from its draft");
    const reopenedId = await evaluate(`fetch("/api/documents/3").then((r) => r.ok ? 3 : 0)`);
    check(reopenedId === 3 && (await text("document-name")) === "word-sample.docx", "the draft is open under the same name");
    check(await waitFor(`fetch("/api/documents/3").then((r) => r.json()).then((d) => JSON.stringify(d.model.blocks).includes("Draft check"))`), "the reopened document has the earlier edits");

    // A crash or closed tab: after reloading, the draft is offered and recovered.
    await cdp.send("Page.enable");
    cdp.on("Page.javascriptDialogOpening", () => cdp.send("Page.handleJavaScriptDialog", { accept: true }).catch(() => {}));
    await sleep(2500); // let the draft catch up
    await cdp.send("Page.reload", { ignoreCache: true });
    check(await waitFor(`document.getElementById("status-connection")?.dataset.state === "connected" && !document.getElementById("recovery").hidden`, 15000), "after a restart, recovery of the draft is offered");
    check((await text("recovery-text")).includes("word-sample.docx"), "the offer names the document");
    await shot("recovery-offer");
    await shot("recovery-offer-narrow", 420, 860);
    await evaluate(`document.getElementById("recovery-open").click()`);
    check(await waitFor(`document.getElementById("document-name").textContent === "word-sample.docx" && document.getElementById("save-state").textContent === "Unsaved changes"`), "Recover opens the draft with unsaved changes");
    check(await waitFor(`document.getElementById("doc-content").textContent.includes("Draft check")`), "the recovered document has the edits");

    // The server can't be reached for a while: typing is kept and sent later.
    const recoveredId = await evaluate(`fetch("/api/documents/4").then((r) => (r.ok ? 4 : 0))`);
    await cdp.send("Network.enable");
    await cdp.send("Network.emulateNetworkConditions", { offline: true, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
    const recoveredTitle = await evaluate(`Number(document.querySelector("#doc-content .para-title").dataset.block)`);
    await caretAt(recoveredTitle, 0);
    await type("Offline ");
    check(await waitFor(`document.getElementById("notice-text").textContent.includes("can't be reached")`), "typing while the server is unreachable is kept, with a notice");
    check((await evaluate(`document.querySelector("#doc-content .para-title").textContent`)).startsWith("Offline "), "the typed text stays on the page");
    await cdp.send("Network.emulateNetworkConditions", { offline: false, latency: 0, downloadThroughput: -1, uploadThroughput: -1 });
    check(
      recoveredId === 4 && (await waitFor(`fetch("/api/documents/4").then((r) => r.json()).then((d) => JSON.stringify(d.model.blocks).includes("Offline Draft check"))`, 15000)),
      "the kept typing is sent when the server is reachable again",
    );

    cdp.close();
  } finally {
    await session.close();
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
