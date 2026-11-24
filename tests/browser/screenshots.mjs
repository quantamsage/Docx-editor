// Takes the screenshots shown in the README (docs/screenshots/) from the
// real interface in headless Edge/Chrome, using the Word-made sample
// document (tests/fixtures/word-sample.docx; no personal content).
//
//   cargo build --release
//   node tests/browser/screenshots.mjs [output folder]

import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { ROOT, launch, sleep } from "./harness.mjs";

const OUT = process.argv[2] || join(ROOT, "docs", "screenshots");
const FIXTURE = fileURLToPath(new URL("../fixtures/word-sample.docx", import.meta.url));

async function main() {
  mkdirSync(OUT, { recursive: true });
  const session = await launch({ port: 9336, name: "screenshots" });
  const { cdp, evaluate, waitFor, press, type, drop, url } = session;
  const size = (width, height) => cdp.send("Emulation.setDeviceMetricsOverride", { width, height, deviceScaleFactor: 1, mobile: width < 600 });
  const scheme = (value) => cdp.send("Emulation.setEmulatedMedia", { features: [{ name: "prefers-color-scheme", value }] });
  const shot = async (name) => {
    await sleep(500);
    const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
    writeFileSync(join(OUT, `${name}.png`), Buffer.from(data, "base64"));
    console.log(`saved ${name}.png`);
  };
  /** Puts the caret in the paragraph matched by `selector`, at `offset`. */
  const caret = (selector, offset = 0) =>
    evaluate(`(() => {
      const p = document.querySelector(${JSON.stringify(selector)});
      p.closest("[contenteditable=true]").focus({ preventScroll: true });
      const node = document.createTreeWalker(p, NodeFilter.SHOW_TEXT).nextNode();
      const range = document.createRange();
      range.setStart(node || p, node ? Math.min(${offset}, node.data.length) : 0);
      getSelection().removeAllRanges();
      getSelection().addRange(range);
      document.dispatchEvent(new Event("selectionchange"));
    })()`);
  const scrollTo = (selector, block = "start") => evaluate(`document.querySelector(${JSON.stringify(selector)}).scrollIntoView({ block: "${block}" })`);
  try {
    await size(1280, 860);
    await scheme("light");
    await cdp.send("Page.navigate", { url });
    await waitFor(`document.getElementById("status-connection")?.dataset.state === "connected"`);
    await shot("welcome");

    await drop(readFileSync(FIXTURE).toString("base64"), "word-sample.docx");
    await waitFor(`[...document.querySelectorAll("#doc-content .obj-image img")].some((img) => img.complete && img.naturalWidth > 0)`);
    // The document, with the cursor in a bold word.
    await caret("#doc-content > [data-block='2']", 16);
    await scrollTo("#workspace .page");
    await shot("editor");

    // A table cell: the table toolbar appears.
    await caret("#doc-content td[data-row='1'][data-cell='1'] [data-kind='paragraph']", 1);
    await scrollTo("#doc-content .table-wrap", "center");
    await shot("table-and-picture");

    // The header, being edited, and the footer's page number.
    await caret("#doc-header [data-kind='paragraph']", 7);
    await scrollTo("#workspace .page");
    await shot("header");
    await caret("#doc-footer [data-kind='paragraph']", 0);
    await scrollTo("#doc-footer", "end");
    await shot("footer-page-number");

    // Find and replace.
    await caret("#doc-content > [data-block='2']", 0);
    await press("h", "KeyH", 72, 2);
    await type("regions");
    await evaluate(`document.getElementById("replace-input").value = "markets"`);
    await waitFor(`/of/.test(document.getElementById("find-count").textContent)`);
    await shot("find-and-replace");
    await press("Escape", "Escape", 27);

    // Page setup.
    await caret("#doc-content > [data-block='2']", 16);
    await scrollTo("#workspace .page");
    await evaluate(`document.querySelector('[data-insert="page-setup"]').click()`);
    await waitFor(`document.getElementById("page-setup-dialog").open`);
    await shot("page-setup");
    await press("Escape", "Escape", 27);

    // Dark interface (the sheet stays white, as on paper).
    await scheme("dark");
    await caret("#doc-content > [data-block='2']", 16);
    await scrollTo("#workspace .page");
    await shot("editor-dark");

    // Phone width, light and dark.
    await size(390, 844);
    await scheme("light");
    await caret("#doc-content > [data-block='2']", 16);
    await scrollTo("#workspace .page");
    await shot("phone");
    await scheme("dark");
    await shot("phone-dark");
    await scheme("light");
    await evaluate(`document.querySelector('[data-insert="page-setup"]').click()`);
    await waitFor(`document.getElementById("page-setup-dialog").open`);
    await shot("phone-page-setup");
  } finally {
    await session.close();
  }
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
