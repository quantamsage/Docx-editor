import { test } from "node:test";
import assert from "node:assert/strict";

import { containerKey } from "../../web/js/editor/containers.js";
import { findMatches, matchLabel, replaceEdits, replaceInParagraph, searchPattern, searchableParagraphs } from "../../web/js/editor/find.js";

const run = (text, props = {}) => ({ type: "run", props, content: [{ type: "text", text }] });
const para = (...inlines) => ({ type: "paragraph", inlines });
const picture = { type: "run", props: {}, content: [{ type: "image", rel_id: "rId1" }] };

const model = {
  blocks: [
    para(run("The cat sat. "), run("Cat", { bold: true }), run(" and concatenate")),
    { type: "table", rows: [{ cells: [{ span: 1, blocks: [para(run("cat in a cell"))] }] }] },
    para(run("A "), picture, run(" cat picture")),
  ],
  header: { part: "word/header1.xml", blocks: [para(run("Cat header"))] },
};

test("paragraphs are searched in the body, table cells, then header and footer", () => {
  const list = searchableParagraphs(model);
  assert.deepEqual(list.map((p) => [p.key, p.block]), [
    ["", 0],
    [containerKey({ cell: { table: 1, row: 0, cell: 0 } }), 0],
    ["", 2],
    [containerKey({ part: "word/header1.xml" }), 0],
  ]);
});

test("matching: case, whole words, special characters", () => {
  assert.equal(findMatches(model, "cat").length, 6, "case-insensitive by default, inside words too");
  assert.equal(findMatches(model, "cat", { matchCase: true }).length, 4);
  const whole = findMatches(model, "cat", { wholeWord: true });
  assert.equal(whole.length, 5, "not inside 'concatenate'");
  assert.deepEqual(whole[0], { key: "", block: 0, start: 4, end: 7 });
  assert.deepEqual(whole[1], { key: "", block: 0, start: 13, end: 16 }, "spans the bold run");
  assert.deepEqual(findMatches(model, "cat", { wholeWord: true })[3], { key: "", block: 2, start: 4, end: 7 }, "the picture counts as one character");
  assert.equal(findMatches(model, "sat.").length, 1, "regular-expression characters are literal");
  assert.equal(findMatches(model, "s.t").length, 0);
  assert.equal(searchPattern(""), null);
  assert.equal(searchPattern("a\nb"), null);
  assert.equal(searchPattern("x".repeat(256)), null);
  assert.deepEqual(findMatches(model, "ÉTÉ"), []);
  assert.equal(findMatches({ blocks: [para(run("Été"))] }, "été").length, 1, "Unicode case folding");
});

test("replacing keeps the formatting around matches and objects", () => {
  const p = model.blocks[0];
  const ranges = [{ start: 4, end: 7 }, { start: 13, end: 16 }];
  assert.deepEqual(replaceInParagraph(0, p, ranges, "dog"), [
    { type: "text", text: "The dog sat. ", like: { block: 0, inline: 0 } },
    { type: "text", text: "dog", like: { block: 0, inline: 1 } },
    { type: "keep", block: 0, inline: 2 },
  ]);
  // A match across two runs takes the formatting where it starts.
  assert.deepEqual(replaceInParagraph(0, p, [{ start: 10, end: 15 }], "X"), [
    { type: "text", text: "The cat saX", like: { block: 0, inline: 0 } },
    { type: "text", text: "t", like: { block: 0, inline: 1 } },
    { type: "keep", block: 0, inline: 2 },
  ]);
  assert.deepEqual(replaceInParagraph(2, model.blocks[2], [{ start: 4, end: 7 }], ""), [
    { type: "keep", block: 2, inline: 0 },
    { type: "keep", block: 2, inline: 1 },
    { type: "text", text: "  picture", like: { block: 2, inline: 2 } },
  ]);
});

test("replace all is one splice per paragraph, in each container", () => {
  const edits = replaceEdits(model, findMatches(model, "cat", { wholeWord: true }), "dog");
  assert.equal(edits.length, 4);
  assert.deepEqual(edits.map((e) => [e.container || null, e.first]), [
    [null, 0],
    [{ cell: { table: 1, row: 0, cell: 0 } }, 0],
    [null, 2],
    [{ part: "word/header1.xml" }, 0],
  ]);
  assert.ok(edits.every((e) => e.op === "splice" && e.first === e.last));
  assert.deepEqual(edits[3].content, [{ type: "text", text: "dog header", like: { block: 0, inline: 0 } }]);
});

test("match labels", () => {
  assert.equal(matchLabel(0, 0), "No results");
  assert.equal(matchLabel(2, 12), "3 of 12");
  assert.equal(matchLabel(-1, 1), "1 result");
});
