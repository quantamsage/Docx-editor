import { test } from "node:test";
import assert from "node:assert/strict";

import { bulletGlyph, formatAt, formatCounter, formatRange, listKind, listMarkers, paragraphSegments, wordAt } from "../../web/js/editor/content.js";

const run = (text, props = {}) => ({ type: "run", props, content: [{ type: "text", text }] });
const revenue = { inlines: [run("Revenue grew "), run("12%", { bold: true }), run(" this quarter", { italic: true })] };
const link = { inlines: [run("Visit "), { type: "hyperlink", url: "https://example.com", runs: [run("site")] }, run(" now")] };
const para = (block, paragraph) => ({ block, paragraph, segments: paragraphSegments(paragraph) });

test("formatting part of a run rewrites only the selected text", () => {
  // Select "grew" in "Revenue grew ".
  const [splice] = formatRange([para(1, revenue)], 8, 12, { bold: true });
  assert.deepEqual(splice, {
    first: 1,
    last: 1,
    content: [
      { type: "text", text: "Revenue ", like: { block: 1, inline: 0 } },
      { type: "text", text: "grew", format: { bold: true }, like: { block: 1, inline: 0 } },
      { type: "text", text: " ", like: { block: 1, inline: 0 } },
      { type: "keep", block: 1, inline: 1 },
      { type: "keep", block: 1, inline: 2 },
    ],
  });
});

test("formatting a whole run keeps its neighbours untouched", () => {
  const [splice] = formatRange([para(1, revenue)], 13, 16, { color: "FF0000" });
  assert.deepEqual(splice.content, [
    { type: "keep", block: 1, inline: 0 },
    { type: "text", text: "12%", format: { color: "FF0000" }, like: { block: 1, inline: 1 } },
    { type: "keep", block: 1, inline: 2 },
  ]);
});

test("objects inside the selection are kept, not reformatted", () => {
  const [splice] = formatRange([para(0, link)], 0, 11, { italic: true });
  assert.deepEqual(splice.content.map((p) => p.type), ["text", "keep", "text"]);
  assert.deepEqual(splice.content[1], { type: "keep", block: 0, inline: 1 });
  assert.deepEqual(splice.content[2], { type: "text", text: " now", format: { italic: true }, like: { block: 0, inline: 2 } });
});

test("a selection across paragraphs formats each one separately", () => {
  const second = { inlines: [run("Second line")] };
  const splices = formatRange([para(1, revenue), para(2, second)], 13, 6, { underline: true });
  assert.equal(splices.length, 2);
  assert.deepEqual(splices[0].content.slice(0, 1), [{ type: "keep", block: 1, inline: 0 }]);
  assert.deepEqual(splices[1], {
    first: 2,
    last: 2,
    content: [
      { type: "text", text: "Second", format: { underline: true }, like: { block: 2, inline: 0 } },
      { type: "text", text: " line", like: { block: 2, inline: 0 } },
    ],
  });
  // Paragraphs with nothing selected (empty range) produce no splice.
  assert.equal(formatRange([para(1, revenue)], 5, 5, { bold: true }).length, 0);
});

test("words and formatting at the caret", () => {
  const segs = paragraphSegments(revenue);
  assert.deepEqual(wordAt(segs, 10), [8, 12]);
  assert.deepEqual(wordAt(segs, 14), [13, 15], "digits and letters form words");
  assert.deepEqual(wordAt(segs, 12), [8, 12], "a caret at the end of a word selects that word");
  assert.equal(wordAt(paragraphSegments({ inlines: [run("a  b")] }), 2), null, "between spaces: no word");
  assert.equal(wordAt(paragraphSegments({ inlines: [] }), 0), null);

  assert.deepEqual(formatAt(revenue, segs, 14), { bold: true });
  assert.deepEqual(formatAt(revenue, segs, 13), {}, "end of the first run");
  assert.deepEqual(formatAt(revenue, segs, 0), {}, "start of the paragraph: the first run");
  assert.deepEqual(formatAt({ inlines: [run("x", { italic: true })] }, paragraphSegments({ inlines: [run("x", { italic: true })] }), 0), { italic: true });
});

test("list counters and markers", () => {
  assert.deepEqual([1, 2, 26, 27, 28].map((n) => formatCounter(n, "lowerLetter")), ["a", "b", "z", "aa", "ab"]);
  assert.deepEqual([1, 4, 9, 14, 1994].map((n) => formatCounter(n, "upperRoman")), ["I", "IV", "IX", "XIV", "MCMXCIV"]);
  assert.equal(formatCounter(7, "decimalZero"), "07");

  const lists = [
    { num_id: "1", levels: [{ format: "decimal", text: "%1.", start: 1 }, { format: "lowerLetter", text: "%2)", start: 1 }, { format: "decimal", text: "%1.%2.%3", start: 1 }] }, // each %k uses its level's format
    { num_id: "2", levels: [{ format: "bullet", text: "•", start: 1 }] },
  ];
  const p = (num_id, level) => ({ type: "paragraph", inlines: [], numbering: num_id ? { num_id, level } : undefined });
  const model = {
    lists,
    blocks: [p("1", 0), p("1", 1), p("1", 1), p("1", 2), p("2", 0), p(null), p("1", 0), p("1", 1), { type: "table" }, p("9", 0)],
  };
  assert.deepEqual(listMarkers(model), { 0: "1.", 1: "a)", 2: "b)", 3: "1.b.1", 4: "•", 6: "2.", 7: "a)" });
  assert.equal(listKind(model, model.blocks[4]), "bullet");
  assert.equal(listKind(model, model.blocks[0]), "number");
  assert.equal(listKind(model, model.blocks[5]), null);
});

test("Word's symbol-font bullets are shown as Unicode", () => {
  assert.equal(bulletGlyph(""), "•");
  assert.equal(bulletGlyph(""), "▪");
  assert.equal(bulletGlyph(""), "✓");
  assert.equal(bulletGlyph(""), "•", "unknown private-use glyphs fall back to a bullet");
  assert.equal(bulletGlyph("o"), "o", "ordinary characters are kept");
  assert.equal(bulletGlyph(""), "•");
});
