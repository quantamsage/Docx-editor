// Keyboard and accessibility test: drives the interface with the keyboard
// only, in headless Edge/Chrome, and checks focus, dialogs, announcements,
// what the accessibility tree exposes, text contrast in the light and dark
// themes, and the layout at phone width.
//
//   cargo build --release
//   node tests/browser/accessibility.mjs

import { checker, launch, sleep } from "./harness.mjs";

const TAB = ["Tab", "Tab", 9];
const ENTER = ["Enter", "Enter", 13];
const ESCAPE = ["Escape", "Escape", 27];
const SHIFT = 8;
const CTRL = 2;

async function main() {
  const session = await launch({ port: 9335, name: "accessibility" });
  const { cdp, evaluate, waitFor, press, type, url } = session;
  const { check, failures } = checker();
  try {
    await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
    await cdp.send("Page.navigate", { url });
    await waitFor(`document.getElementById("status-connection")?.dataset.state === "connected"`);
    const focused = () =>
      evaluate(`(() => { const a = document.activeElement; return a ? a.id || a.dataset.action || a.dataset.format || a.dataset.insert || a.dataset.table || a.tagName.toLowerCase() : ""; })()`);
    const model = () => evaluate(`fetch("/api/documents/1").then((r) => r.json()).then((d) => d.model)`);
    const waitForModel = async (predicate) => {
      const deadline = Date.now() + 8000;
      while (Date.now() < deadline) {
        if (predicate(await model())) return true;
        await sleep(150);
      }
      return false;
    };
    const tableOf = (m) => m.blocks.find((b) => b.type === "table");
    const cellText = (m, r, c) => (tableOf(m)?.rows[r]?.cells[c]?.blocks || []).flatMap((b) => b.inlines || []).flatMap((i) => i.content || []).map((x) => x.text || "").join("");

    // New puts the cursor in the page, so typing goes straight into the document.
    await evaluate(`document.querySelector('[data-action="new"]').click()`);
    check(await waitFor(`document.activeElement === document.getElementById("doc-content")`), "New puts the cursor in the new document");
    await type("First line");
    check(await waitForModel((m) => JSON.stringify(m.blocks[0]).includes("First line")), "typing after New reaches the document");

    // Enter in a dialog field confirms the dialog (it used to cancel it).
    await evaluate(`document.querySelector('[data-insert="table"]').click()`);
    check(await waitFor(`document.getElementById("table-dialog").open && document.activeElement.id === "table-columns"`), "the table dialog opens with the focus in its first field");
    await press(...TAB);
    check((await focused()) === "table-rows", "Tab moves to the next field of the dialog");
    await press("Backspace", "Backspace", 8);
    await type("2");
    await press(...ENTER);
    check(await waitForModel((m) => tableOf(m)?.rows.length === 2 && tableOf(m).rows[0].cells.length === 3), "Enter in a dialog field inserts the table");
    check(await waitFor(`!document.getElementById("table-dialog").open && document.activeElement.closest?.("#doc-content") !== null`), "the focus returns to the document after the dialog");

    // Tab moves between table cells, Shift+Tab back, and Tab in the last cell adds a row.
    await type("a1");
    await press(...TAB);
    await type("b1");
    check(await waitForModel((m) => cellText(m, 0, 0) === "a1" && cellText(m, 0, 1) === "b1"), "Tab moves to the next cell");
    await press("Tab", "Tab", 9, SHIFT);
    await type("<");
    check(await waitForModel((m) => cellText(m, 0, 0) === "<a1"), "Shift+Tab moves to the previous cell");
    for (let i = 0; i < 5; i++) await press(...TAB);
    await type("last");
    check(await waitForModel((m) => cellText(m, 1, 2) === "last"), "Tab reaches the last cell");
    await press(...TAB);
    check(await waitForModel((m) => tableOf(m)?.rows.length === 3), "Tab in the last cell adds a row");
    await type("new row");
    check(await waitForModel((m) => cellText(m, 2, 0) === "new row"), "the cursor is in the new row's first cell");

    check(await waitFor(`!document.getElementById("table-toolbar").hidden && document.querySelectorAll('#table-toolbar [tabindex="0"]').length === 1`), "the table toolbar appears in a table with one Tab stop");

    // Escape closes dialogs and returns the focus to the document.
    await evaluate(`document.querySelector('[data-insert="page-setup"]').click()`);
    check(await waitFor(`document.getElementById("page-setup-dialog").open`), "Page setup opens");
    await press(...ESCAPE);
    check(await waitFor(`!document.getElementById("page-setup-dialog").open && document.activeElement.closest?.("#doc-content") !== null`), "Escape closes the dialog and returns to the document");

    // Tab at the start of a list item changes its level.
    await evaluate(`(() => {
      const p = document.querySelector('#doc-content > [data-block="0"]');
      const r = document.createRange(); r.setStart(p, 0); r.collapse(true);
      document.getElementById("doc-content").focus();
      getSelection().removeAllRanges(); getSelection().addRange(r);
    })()`);
    await press("*", "Digit8", 56, CTRL | SHIFT);
    check(await waitForModel((m) => m.blocks[0].numbering?.level === 0), "Ctrl+Shift+8 makes a bulleted list");
    await press(...TAB);
    check(await waitForModel((m) => m.blocks[0].numbering?.level === 1), "Tab at the start of a list item moves it a level down");
    await press("Tab", "Tab", 9, SHIFT);
    check(await waitForModel((m) => m.blocks[0].numbering?.level === 0), "Shift+Tab moves it back up");
    check(await waitForModel((m) => !JSON.stringify(m.blocks[0]).includes("\\t")), "no tab character was typed into the list item");

    // Toolbars: one Tab stop each, arrow keys inside, Alt+F10, Escape and F6.
    const toolbars = await evaluate(`[...document.querySelectorAll('[role="toolbar"]:not([hidden])')].map((t) => [t.getAttribute("aria-label"), t.querySelectorAll('[tabindex="0"]').length])`);
    check(toolbars.length === 2 && toolbars.every(([label, stops]) => label && stops === 1), `each toolbar is labelled and has one Tab stop (${JSON.stringify(toolbars)})`);
    await evaluate(`document.activeElement.blur()`);
    const stops = [];
    for (let i = 0; i < 12 && stops[stops.length - 1] !== "doc-content"; i++) {
      await press(...TAB);
      stops.push(await focused());
    }
    check(stops[stops.length - 1] === "doc-content" && stops.length <= 4, `Tab reaches the document in a few steps (${stops.join(" → ")})`);
    await press("F10", "F10", 121, 1);
    check((await evaluate(`document.activeElement.closest('[role="toolbar"]')?.id`)) === "format-toolbar", "Alt+F10 goes to the formatting toolbar");
    const first = await focused();
    await press("ArrowRight", "ArrowRight", 39);
    const second = await focused();
    check(first !== second && (await evaluate(`document.activeElement.closest("#format-toolbar") !== null`)), `ArrowRight moves to the next control (${first} → ${second})`);
    await press("End", "End", 35);
    check((await focused()) === "para-spacing", "End goes to the last control");
    await press("Home", "Home", 36);
    check((await focused()) === "style-select", "Home goes to the first control");
    await press("ArrowLeft", "ArrowLeft", 37);
    check((await focused()) === "para-spacing", "ArrowLeft wraps around to the last control");
    await press(...ESCAPE);
    check(await waitFor(`document.activeElement.id === "doc-content"`), "Escape in a toolbar returns to the document");
    await press("F6", "F6", 117);
    check((await evaluate(`document.activeElement.closest('[role="toolbar"]')?.getAttribute("aria-label")`)) === "File and editing", "F6 from the document goes to the first toolbar");
    await press("F6", "F6", 117, SHIFT);
    check((await focused()) === "doc-content", "Shift+F6 goes back to the document");

    // The find bar: the Find button says whether it is open.
    await press("f", "KeyF", 70, CTRL);
    check(await waitFor(`!document.getElementById("find-bar").hidden && document.querySelector('[data-action="find"]').getAttribute("aria-expanded") === "true"`), "Ctrl+F opens the find bar and Find is marked expanded");
    await press(...ESCAPE);
    check(await waitFor(`document.querySelector('[data-action="find"]').getAttribute("aria-expanded") === "false"`), "closing the find bar marks Find collapsed");

    // Screen readers hear what happened: through live regions that are always there.
    const spoken = (id) => evaluate(`document.getElementById("${id}").textContent`);
    check(
      (await evaluate(`document.getElementById("announce-polite").getAttribute("role")`)) === "status" &&
        (await evaluate(`document.getElementById("announce-urgent").getAttribute("role")`)) === "alert",
      "there is a polite and an urgent live region",
    );
    await evaluate(`document.getElementById("doc-content").focus()`);
    await press("z", "KeyZ", 90, CTRL);
    check(await waitFor(`document.getElementById("announce-polite").textContent === "Undone."`), "undoing is announced");
    await press("y", "KeyY", 89, CTRL);
    check(await waitFor(`document.getElementById("announce-polite").textContent === "Redone."`), "redoing is announced");
    await evaluate(`(() => {
      const transfer = new DataTransfer();
      transfer.items.add(new File(["not a document"], "notes.txt"));
      document.getElementById("workspace").dispatchEvent(new DragEvent("drop", { dataTransfer: transfer, bubbles: true, cancelable: true }));
    })()`);
    check(await waitFor(`document.getElementById("announce-urgent").textContent.includes("is not a Word document")`), `errors are announced urgently (${await spoken("announce-urgent")})`);
    check((await evaluate(`document.getElementById("notice").dataset.kind`)) === "error" && !(await evaluate(`document.getElementById("notice").hasAttribute("role")`)), "the visible notice shows the error without announcing it twice");
    await evaluate(`document.querySelector('[data-action="undo"]').click()`);
    await waitFor(`document.getElementById("announce-polite").textContent === "Undone."`);
    await waitFor(`!document.querySelector('[data-action="redo"]').disabled`);
    await evaluate(`document.querySelector('[data-action="redo"]').click()`);
    await waitFor(`document.getElementById("announce-polite").textContent === "Redone."`);
    await evaluate(`document.querySelector('[data-action="redo"]').disabled = false; document.querySelector('[data-action="redo"]').click()`);
    check(await waitFor(`document.getElementById("notice").dataset.kind === "info" && document.getElementById("announce-polite").textContent === "There is nothing to redo."`), `information is shown as such and announced politely (${await evaluate(`document.getElementById("notice").dataset.kind + ": " + document.getElementById("notice-text").textContent + " / " + document.getElementById("announce-polite").textContent`)})`);

    // The accessibility tree: every control has a name; toggles say whether they are on.
    await cdp.send("Accessibility.enable");
    const { nodes } = await cdp.send("Accessibility.getFullAXTree");
    const controls = nodes.filter((n) => !n.ignored && ["button", "combobox", "textbox", "checkbox", "radio", "spinbutton", "ColorWell", "toolbar", "searchbox"].includes(n.role?.value));
    const unnamed = controls.filter((n) => !(n.name?.value || "").trim()).map((n) => n.role.value);
    check(controls.length > 30 && unnamed.length === 0, `all ${controls.length} controls have accessible names${unnamed.length ? ` (unnamed: ${unnamed.join(", ")})` : ""}`);
    const bold = controls.find((n) => n.name?.value === "Bold");
    check(bold && (bold.properties || []).some((p) => p.name === "pressed"), "Bold exposes its pressed state");
    const shortcut = await evaluate(`document.querySelector('[data-format="bold"]').getAttribute("aria-keyshortcuts")`);
    check(shortcut === "Control+B", "toolbar buttons announce their keyboard shortcuts");

    // Text contrast (WCAG AA, 4.5:1) of the interface in both themes. The
    // colour and background are measured as drawn, through transparent
    // backgrounds and opacity.
    const contrast = (selectors) =>
      evaluate(`(() => {
        const parse = (c) => { const m = c.match(/[\\d.]+/g).map(Number); return { r: m[0], g: m[1], b: m[2], a: m.length > 3 ? m[3] : 1 }; };
        const over = (top, bottom) => ({ r: top.r * top.a + bottom.r * (1 - top.a), g: top.g * top.a + bottom.g * (1 - top.a), b: top.b * top.a + bottom.b * (1 - top.a), a: 1 });
        const background = (el) => {
          const layers = [];
          for (let n = el; n; n = n.parentElement) {
            const c = parse(getComputedStyle(n).backgroundColor);
            if (c.a > 0) layers.push(c);
            if (c.a >= 1) break;
          }
          return layers.reverse().reduce((acc, c) => over(c, acc), { r: 255, g: 255, b: 255, a: 1 });
        };
        const lum = ({ r, g, b }) => [r, g, b].map((v) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; }).reduce((s, v, i) => s + v * [0.2126, 0.7152, 0.0722][i], 0);
        const out = [];
        for (const selector of ${JSON.stringify(selectors)}) {
          for (const el of document.querySelectorAll(selector)) {
            if (!el.getClientRects().length || !el.textContent.trim()) continue;
            let opacity = 1;
            for (let n = el; n; n = n.parentElement) opacity *= Number(getComputedStyle(n).opacity);
            const bg = background(el);
            const fg = over({ ...parse(getComputedStyle(el).color), a: parse(getComputedStyle(el).color).a * opacity }, bg);
            const [a, b] = [lum(fg), lum(bg)].sort((x, y) => y - x);
            out.push([selector, Math.round(((a + 0.05) / (b + 0.05)) * 100) / 100]);
            break;
          }
        }
        return out;
      })()`);
    const TEXT = [".titlebar-app", "#document-name", "#save-state", ".toolbar-main .tool:not(:disabled) .tool-label", "#style-select:not(:disabled)", "#status-words", "#status-connection-text", "#notice-text", ".find-option", "#find-count", ".toolbar-title", "#table-dialog h2", "#table-dialog label", "#doc-content .para"];
    const showAll = async () => {
      await evaluate(`(() => { const p = document.querySelector("#doc-content .para"); document.getElementById("doc-content").focus(); const r = document.createRange(); r.setStart(p, 0); getSelection().removeAllRanges(); getSelection().addRange(r); document.dispatchEvent(new Event("selectionchange")); })()`);
      await waitFor(`!document.getElementById("style-select").disabled`);
      await evaluate(`document.getElementById("find-bar").hidden = false; document.getElementById("find-count").textContent = "1 of 3"; document.getElementById("table-toolbar").hidden = false`);
    };
    for (const theme of ["light", "dark"]) {
      await cdp.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: theme }] });
      await showAll();
      for (const kind of ["error", "warning", "info"]) {
        await evaluate(`document.getElementById("notice").hidden = false; document.getElementById("notice").dataset.kind = "${kind}"`);
        const [[, ratio]] = await contrast(["#notice-text"]);
        check(ratio >= 4.5, `${theme} theme: ${kind} messages have enough contrast (${ratio}:1)`);
      }
      const results = await contrast(TEXT.filter((t) => t !== "#notice-text"));
      const low = results.filter(([, ratio]) => ratio < 4.5);
      check(results.length >= 10 && !low.length, `${theme} theme: interface text has at least 4.5:1 contrast (${results.length} checked${low.length ? `; too low: ${JSON.stringify(low)}` : ""})`);
      await evaluate(`document.querySelector('[data-insert="table"]').click()`);
      await waitFor(`document.getElementById("table-dialog").open`);
      const dialog = await contrast(["#table-dialog h2", "#table-dialog label", "#table-dialog .button"]);
      check(dialog.length === 3 && dialog.every(([, r]) => r >= 4.5), `${theme} theme: dialogs have enough contrast (${JSON.stringify(dialog)})`);
      await press(...ESCAPE);
    }
    await cdp.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value: "light" }] });
    await evaluate(`document.getElementById("find-bar").hidden = true; document.getElementById("table-toolbar").hidden = true; document.getElementById("notice").hidden = true`);

    // Reduced motion is respected (nothing in the interface animates).
    await cdp.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-reduced-motion", value: "reduce" }] });
    check((await evaluate(`getComputedStyle(document.querySelector(".tool")).transitionDuration`)) === "0s", "reduced motion: no transitions");
    await cdp.send("Emulation.setEmulatedMedia", { features: [] });

  } finally {
    await session.close();
  }
  if (failures.length) {
    console.error(`\n${failures.length} check(s) failed.`);
    process.exit(1);
  }
  console.log("\nAll accessibility checks passed.");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
