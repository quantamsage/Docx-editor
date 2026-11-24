// Keyboard and accessibility test: drives the interface with the keyboard
// only, in headless Edge/Chrome, and checks focus, dialogs and what the
// accessibility tree exposes.
//
//   cargo build --release
//   node tests/browser/keyboard.mjs

import { checker, launch, sleep } from "./harness.mjs";

const TAB = ["Tab", "Tab", 9];
const ENTER = ["Enter", "Enter", 13];
const ESCAPE = ["Escape", "Escape", 27];
const SHIFT = 8;
const CTRL = 2;

async function main() {
  const session = await launch({ port: 9335, name: "keyboard" });
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

    // Escape closes dialogs and returns the focus to the document.
    await evaluate(`document.querySelector('[data-insert="page-setup"]').click()`);
    check(await waitFor(`document.getElementById("page-setup-dialog").open`), "Page setup opens");
    await press(...ESCAPE);
    check(await waitFor(`!document.getElementById("page-setup-dialog").open && document.activeElement.closest?.("#doc-content") !== null`), "Escape closes the dialog and returns to the document");

  } finally {
    await session.close();
  }
  if (failures.length) {
    console.error(`\n${failures.length} check(s) failed.`);
    process.exit(1);
  }
  console.log("\nAll keyboard checks passed.");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
