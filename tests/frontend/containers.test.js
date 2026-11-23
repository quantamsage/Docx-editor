import { test } from "node:test";
import assert from "node:assert/strict";

import { cellKey, containerBlocks, containerField, containerKey, isHeaderFooterKey, parentKey, parseContainerKey } from "../../web/js/editor/containers.js";
import { describeObject, paragraphSegments, replaceRange } from "../../web/js/editor/content.js";
import { tableLayout } from "../../web/js/editor/render.js";

const run = (text) => ({ type: "run", props: {}, content: [{ type: "text", text }] });
const para = (...texts) => ({ type: "paragraph", inlines: texts.map(run) });
const cell = (blocks, extra = {}) => ({ span: 1, blocks, ...extra });

const model = {
  blocks: [
    para("Intro"),
    { type: "table", rows: [{ cells: [cell([para("A")]), cell([para("B"), para("B2")])] }] },
  ],
  header: { part: "word/header1.xml", blocks: [para("Head"), { type: "table", rows: [{ cells: [cell([para("H cell")])] }] }] },
  footer: { part: "word/footer1.xml", blocks: [para("Foot")] },
};

test("container keys round-trip and address body, cells, headers and footers", () => {
  assert.equal(containerKey({}), "");
  assert.equal(containerKey(null), "");
  const key = containerKey({ cell: { table: 1, row: 0, cell: 1 } });
  assert.deepEqual(parseContainerKey(key), { cell: { table: 1, row: 0, cell: 1 } });
  assert.deepEqual(parseContainerKey(""), {});
  const headerCell = cellKey(containerKey({ part: "word/header1.xml" }), 1, 0, 0);
  assert.deepEqual(parseContainerKey(headerCell), { part: "word/header1.xml", cell: { table: 1, row: 0, cell: 0 } });
  assert.equal(parentKey(headerCell), containerKey({ part: "word/header1.xml" }));
  assert.equal(parentKey(key), "");
  assert.ok(isHeaderFooterKey(headerCell) && !isHeaderFooterKey(key));

  assert.equal(containerBlocks(model, "")[0].inlines[0].content[0].text, "Intro");
  assert.equal(containerBlocks(model, key).length, 2);
  assert.equal(containerBlocks(model, headerCell)[0].inlines[0].content[0].text, "H cell");
  assert.equal(containerBlocks(model, containerKey({ part: "word/footer1.xml" }))[0].inlines[0].content[0].text, "Foot");
  assert.equal(containerBlocks(model, containerKey({ part: "word/header9.xml" })), null);
  assert.equal(containerBlocks(model, containerKey({ cell: { table: 0, row: 0, cell: 0 } })), null, "block 0 is not a table");
  assert.equal(containerBlocks(model, containerKey({ cell: { table: 1, row: 3, cell: 0 } })), null);

  assert.deepEqual(containerField(""), {});
  assert.deepEqual(containerField(key), { container: { cell: { table: 1, row: 0, cell: 1 } } });
});

test("table layout follows column spans and vertical merges", () => {
  // [span 2], [A restart, B], [continue, C], [continue, D], [E, F]
  const table = {
    rows: [
      { cells: [cell([], { span: 2 })] },
      { cells: [cell([], { vertical_merge: "restart" }), cell([])] },
      { cells: [cell([], { vertical_merge: "continue" }), cell([])] },
      { cells: [cell([], { vertical_merge: "continue" }), cell([])] },
      { cells: [cell([]), cell([])] },
    ],
  };
  const { starts, rowSpans } = tableLayout(table);
  assert.deepEqual(starts, [[0], [0, 1], [0, 1], [0, 1], [0, 1]]);
  assert.equal(rowSpans.get("1,0"), 3);
  assert.equal(rowSpans.get("2,0"), 0);
  assert.equal(rowSpans.get("3,0"), 0);
  assert.equal(rowSpans.get("1,1"), 1);
  assert.equal(rowSpans.get("0,0"), 1);
  // A continuation without a cell above it is drawn.
  assert.equal(tableLayout({ rows: [{ cells: [cell([], { vertical_merge: "continue" })] }] }).rowSpans.get("0,0"), 1);
});

test("pictures and page breaks are inserted at the cursor", () => {
  const p = { inlines: [run("Hello world")] };
  const segments = paragraphSegments(p);
  const at = { block: 2, paragraph: p, segments, offset: 5 };
  const picture = { type: "image", image: "img1", width_emu: 9525, height_emu: 9525 };
  const { splice, caret } = replaceRange(at, at, "", [picture]);
  assert.deepEqual(splice.content, [
    { type: "text", text: "Hello", like: { block: 2, inline: 0 } },
    picture,
    { type: "text", text: " world", like: { block: 2, inline: 0 } },
  ]);
  assert.deepEqual(caret, { block: 2, offset: 6 }, "after the picture");

  const pageBreak = replaceRange(at, at, "\n", [{ type: "page_break" }]);
  assert.deepEqual(pageBreak.splice.content.map((c) => c.type), ["text", "page_break", "break", "text"]);
  assert.deepEqual(pageBreak.caret, { block: 3, offset: 0 }, "the next paragraph starts the new page");
});

test("page number fields are shown as fields", () => {
  const field = describeObject({ type: "other", kind: "fldSimple", text: "1", field: "PAGE" });
  assert.deepEqual(field, { kind: "field", label: "1", visible: true, field: "PAGE" });
  assert.equal(describeObject({ type: "other", kind: "fldSimple", text: "", field: "NUMPAGES" }).label, "#");
});
