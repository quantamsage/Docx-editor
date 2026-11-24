// Performance check: opens a long document (about 330 pages of headings,
// formatted paragraphs and tables; see large-document.mjs) in headless
// Edge/Chrome and times what a user does with it.
//
//   cargo build --release
//   npm run test:perf
//
// For each step it prints the wall time and the main-thread time spent in
// long tasks (over 50 ms, which is when the page stops responding). The
// checks are generous budgets that catch regressions, not targets.
// SECTIONS=<n> changes the document's length (default 250).

import { buildLargeDocument } from "./large-document.mjs";
import { checker, launch, sleep } from "./harness.mjs";

const SECTIONS = Number(process.env.SECTIONS || 250);

async function main() {
  const session = await launch({ port: 9334, name: "perf" });
  const { cdp, evaluate, waitFor, press, type, drop, url } = session;
  const { check, failures } = checker();
  const results = [];
  try {
    const t0 = Date.now();
    const bytes = await buildLargeDocument(url, SECTIONS);
    console.log(`built a ${SECTIONS}-section document (${bytes.length} bytes) in ${Date.now() - t0} ms`);

    await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
    await cdp.send("Page.navigate", { url });
    await waitFor(`document.getElementById("status-connection")?.dataset.state === "connected"`);
    // Long tasks are collected in the page from here on.
    await evaluate(`(() => {
      window.__long = [];
      new PerformanceObserver((list) => window.__long.push(...list.getEntries().map((e) => e.duration))).observe({ type: "longtask", buffered: false });
    })()`);

    /** Runs `action`, waits for `done` (an expression) and the page to settle, and records the times. */
    const measure = async (label, action, done, budgetMs) => {
      await evaluate(`window.__long.length = 0`);
      const start = Date.now();
      await action();
      const finished = await waitFor(done, 60000);
      const wall = Date.now() - start;
      await sleep(300); // long tasks are reported after they end
      const long = await evaluate(`({ total: window.__long.reduce((a, b) => a + b, 0), max: Math.max(0, ...window.__long) })`);
      results.push({ step: label, wall_ms: wall, long_tasks_ms: Math.round(long.total), longest_ms: Math.round(long.max) });
      check(finished && wall <= budgetMs, `${label}: ${wall} ms (budget ${budgetMs} ms)`);
      return wall;
    };

    await measure(
      "open",
      () => drop(bytes.toString("base64"), "large.docx"),
      `document.getElementById("document-name").textContent === "large.docx" && document.querySelectorAll("#doc-content > [data-block]").length > 3000`,
      8000,
    );
    // Off-screen blocks skip layout once the page settled, keeping their measured height.
    await waitFor(`document.getElementById("doc-content").classList.contains("is-settled")`);
    const skipping = await evaluate(`getComputedStyle(document.querySelector("#doc-content > .para:not(.para-list)")).contentVisibility`);
    check(skipping === "auto", `off-screen blocks skip layout (content-visibility: ${skipping})`);
    await evaluate(`document.dispatchEvent(new Event("selectionchange"))`);
    await sleep(300);
    const pages = Number((await evaluate(`document.getElementById("status-page").textContent`)).split(" of ")[1]);
    check(pages >= SECTIONS, `the page count is still estimated from real heights (${pages} pages)`);
    const blocks = await evaluate(`document.querySelectorAll("#doc-content > [data-block]").length`);
    const docId = await evaluate(`fetch("/api/documents/2").then((r) => (r.ok ? 2 : 1))`);
    const middle = await evaluate(`(() => {
      const ps = [...document.querySelectorAll("#doc-content > .para:not(.para-heading-1)")];
      return Number(ps[Math.floor(ps.length / 2)].dataset.block);
    })()`);
    const caretIn = (block, offset) =>
      evaluate(`(() => {
        const p = document.querySelector('#doc-content > [data-block="${block}"]');
        document.getElementById("doc-content").focus();
        p.scrollIntoView({ block: "center" });
        const node = document.createTreeWalker(p, NodeFilter.SHOW_TEXT).nextNode();
        const range = document.createRange();
        range.setStart(node, ${offset});
        getSelection().removeAllRanges();
        getSelection().addRange(range);
      })()`);
    const paragraphText = (block) => `fetch("/api/documents/${docId}").then((r) => r.json()).then((d) => (d.model.blocks[${block}].inlines || []).flatMap((i) => i.content || []).map((c) => c.text || "").join(""))`;

    // Typing: 20 characters, one key at a time, as fast as a quick typist.
    await caretIn(middle, 0);
    await sleep(500);
    await measure(
      "type 20 characters (until saved on the server)",
      async () => {
        for (const ch of "Performance check 1 ") {
          await type(ch);
          await sleep(60);
        }
      },
      `${paragraphText(middle)}.then((t) => t.startsWith("Performance check 1 "))`,
      5000,
    );
    await measure(
      "Enter (split a paragraph)",
      () => press("Enter", "Enter", 13),
      `document.querySelectorAll("#doc-content > [data-block]").length === ${blocks + 1} && document.activeElement.id === "doc-content"`,
      3000,
    );
    await caretIn(middle, 3);
    await measure(
      "Bold (Ctrl+B on a word)",
      () => press("b", "KeyB", 66, 2),
      `document.querySelector('[data-format="bold"]').getAttribute("aria-pressed") === "true"`,
      3000,
    );
    await measure(
      "Undo (Ctrl+Z)",
      () => press("z", "KeyZ", 90, 2),
      `document.querySelector('[data-format="bold"]').getAttribute("aria-pressed") === "false"`,
      3000,
    );
    await measure(
      "Find a common word (Ctrl+F, type 'growth')",
      async () => {
        await press("f", "KeyF", 70, 2);
        await type("growth");
      },
      `/ of \\d+/.test(document.getElementById("find-count").textContent)`,
      3000,
    );
    await measure("Next match (Enter in find)", () => press("Enter", "Enter", 13), `/^2 of/.test(document.getElementById("find-count").textContent)`, 2000);
    await press("Escape", "Escape", 27);
    await measure(
      "Save (prepare the .docx)",
      () => evaluate(`window.__saved = fetch("/api/documents/${docId}/download").then((r) => r.arrayBuffer()).then((b) => b.byteLength)`),
      `window.__saved.then((n) => n > 100000)`,
      3000,
    );
    await measure(
      "Scroll to the end",
      () => evaluate(`document.getElementById("workspace").scrollTop = 1e9`),
      `document.getElementById("status-page").textContent !== ""`,
      1000,
    );
    console.log();
    console.table(results);
  } finally {
    await session.close();
  }
  if (failures.length) {
    console.error(`\n${failures.length} check(s) failed.`);
    process.exit(1);
  }
  console.log("\nAll performance checks passed.");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
