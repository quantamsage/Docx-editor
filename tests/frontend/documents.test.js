import { test } from "node:test";
import assert from "node:assert/strict";

import {
  ensureDocxExtension,
  outline,
  pageGeometry,
  paragraphText,
  preservedNote,
  summarize,
  twipsToPx,
} from "../../web/js/documents.js";

const run = (text, props = {}) => ({ type: "run", props, content: [{ type: "text", text }] });
const paragraph = (inlines, style_id) => ({ type: "paragraph", style_id, inlines });

test("twipsToPx converts at 96 px per inch", () => {
  assert.equal(twipsToPx(1440), 96);
  assert.equal(twipsToPx(720), 48);
});

test("pageGeometry recognises standard paper sizes and orientation", () => {
  const a4 = pageGeometry({ page_width: 11906, page_height: 16838, margin_top: 1440, margin_right: 1440, margin_bottom: 1440, margin_left: 1440 });
  assert.equal(a4.label, "A4, portrait");
  assert.equal(Math.round(a4.widthPx), 794);
  assert.equal(Math.round(a4.heightPx), 1123);
  assert.deepEqual(a4.margins, { top: 96, right: 96, bottom: 96, left: 96, header: 47.2, footer: 47.2 }, "header and footer default to 1.25 cm");

  const letterLandscape = pageGeometry({ page_width: 15840, page_height: 12240, landscape: true });
  assert.equal(letterLandscape.label, "Letter, landscape");
  assert.equal(letterLandscape.widthPx, 1056);

  // orient="landscape" with portrait-ordered dimensions is swapped, as Word does.
  const swapped = pageGeometry({ page_width: 12240, page_height: 15840, landscape: true });
  assert.equal(swapped.widthPx, 1056);
  assert.equal(swapped.label, "Letter, landscape");

  assert.equal(pageGeometry({ page_width: 7200, page_height: 10080 }).label, "127 × 178 mm, portrait");
});

test("pageGeometry defaults to A4 with 1-inch margins", () => {
  for (const section of [null, undefined, {}, { page_width: null, margin_left: null }]) {
    const g = pageGeometry(section);
    assert.equal(g.label, "A4, portrait");
    assert.equal(g.margins.left, 96);
  }
  // Negative top/bottom margins (text may overlap headers) still lay out.
  assert.equal(pageGeometry({ margin_top: -1440 }).margins.top, 96);
});

test("paragraphText joins runs, hyperlinks and preserved text", () => {
  const p = paragraph([
    run("Hello "),
    { type: "hyperlink", url: "https://example.com", runs: [run("world")] },
    { type: "run", props: {}, content: [{ type: "tab" }, { type: "text", text: "a" }, { type: "line_break" }, { type: "image", rel_id: "rId1" }] },
    { type: "other", kind: "sdt", text: "!" },
  ]);
  assert.equal(paragraphText(p), "Hello world\ta\n!");
  assert.equal(paragraphText({ type: "paragraph", inlines: [] }), "");
});

test("summarize lists non-zero counts", () => {
  const model = {
    stats: { paragraphs: 12, tables: 1, images: 0, hyperlinks: 2, preserved_elements: 0, words: 100 },
    headers: ["word/header1.xml"],
    footers: [],
  };
  assert.deepEqual(summarize(model), ["12 paragraphs", "1 table", "2 hyperlinks", "1 header"]);
  assert.deepEqual(summarize({ stats: {}, headers: [], footers: [] }), []);
});

test("preservedNote explains kept content", () => {
  assert.equal(preservedNote({ stats: { preserved_elements: 0 } }), "");
  assert.match(preservedNote({ stats: { preserved_elements: 1 } }), /^1 element the editor does not display/);
  assert.match(preservedNote({ stats: { preserved_elements: 3 } }), /^3 elements .* kept unchanged/);
});

test("outline finds titles and headings by style id or name", () => {
  const model = {
    styles: [
      { id: "Title", name: "Title" },
      { id: "Heading1", name: "heading 1" },
      { id: "Berschrift2", name: "heading 2" },
      { id: "Normal", name: "Normal" },
    ],
    blocks: [
      paragraph([run("Report")], "Title"),
      paragraph([run("Intro")], "Heading1"),
      paragraph([run("Body text")], "Normal"),
      paragraph([run("Details")], "Berschrift2"),
      paragraph([run("   ")], "Heading1"),
      { type: "table", rows: [] },
      paragraph([run("Plain")]),
    ],
  };
  assert.deepEqual(outline(model), [
    { level: 0, text: "Report" },
    { level: 1, text: "Intro" },
    { level: 2, text: "Details" },
  ]);
  assert.equal(outline(model, 1).length, 1);
});

test("ensureDocxExtension", () => {
  assert.equal(ensureDocxExtension("report"), "report.docx");
  assert.equal(ensureDocxExtension("report.docx"), "report.docx");
  assert.equal(ensureDocxExtension("Macro.DOCM"), "Macro.DOCM");
  assert.equal(ensureDocxExtension("  "), "Untitled document.docx");
  assert.equal(ensureDocxExtension("notes.txt"), "notes.txt.docx");
});
