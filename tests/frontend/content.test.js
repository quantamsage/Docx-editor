import { test } from "node:test";
import assert from "node:assert/strict";

import {
  OBJECT_CHAR,
  describeObject,
  isNoop,
  isTextRun,
  normalizeSegments,
  paragraphSegments,
  remapAfterSplice,
  replaceRange,
  segmentsLength,
  segmentsText,
  segmentsToPieces,
  splitSegments,
} from "../../web/js/editor/content.js";

const run = (text, props = {}) => ({ type: "run", props, content: [{ type: "text", text }] });
const other = (kind) => ({ type: "run", props: {}, content: [{ type: "other", kind }] });

// "Revenue grew " + **12%** + _ this quarter_
const revenue = { inlines: [run("Revenue grew "), run("12%", { bold: true }), run(" this quarter", { italic: true })] };
// "Page " + field (begin, instr, separate, "1", end) + comment reference
const field = {
  inlines: [run("Page "), other("fldChar"), other("instrText"), other("fldChar"), run("1"), other("fldChar"), other("commentReference")],
};
const picture = {
  inlines: [
    { type: "run", props: {}, content: [{ type: "image", rel_id: "rId6", part: "word/media/image1.png", width_emu: 914400, height_emu: 457200, description: "Sales chart" }] },
    run(" Figure 1"),
  ],
};
const link = { inlines: [run("Visit "), { type: "hyperlink", url: "https://example.com", anchor: null, runs: [run("Example site")] }, run(".")] };

const textSeg = (text, inline) => ({ kind: "text", text, inline });
const objSeg = (inline) => ({ kind: "object", inline });

test("segments describe runs as text and everything else as objects", () => {
  const segs = paragraphSegments(revenue);
  assert.deepEqual(segs.map((s) => [s.kind, s.text]), [["text", "Revenue grew "], ["text", "12%"], ["text", " this quarter"]]);
  assert.equal(segs[1].props.bold, true);
  assert.equal(segmentsText(segs), "Revenue grew 12% this quarter");

  const pic = paragraphSegments(picture);
  assert.equal(pic[0].kind, "object");
  assert.equal(pic[0].object.kind, "image");
  assert.equal(pic[0].object.label, "Sales chart");
  assert.equal(segmentsLength(pic), 1 + " Figure 1".length);
  assert.equal(segmentsText(pic)[0], OBJECT_CHAR);

  assert.deepEqual(describeObject(link.inlines[1]), { kind: "link", label: "Example site", visible: true, url: "https://example.com", anchor: null });
  assert.equal(describeObject(field.inlines[1]).visible, false);
  assert.equal(describeObject({ type: "run", content: [{ type: "page_break" }] }).kind, "page_break");
  assert.equal(describeObject({ type: "other", kind: "sdt", text: "Client" }).visible, true);
  assert.ok(isTextRun({ type: "run", content: [{ type: "text", text: "a" }, { type: "tab" }, { type: "line_break" }] }));
  assert.ok(!isTextRun({ type: "run", content: [] }), "empty runs are not editable text");
});

test("unchanged text keeps its runs; edited text is new text formatted like its run", () => {
  const unchanged = normalizeSegments(revenue, [textSeg("Revenue grew ", 0), textSeg("12%", 1), textSeg(" this quarter", 2)]);
  const pieces = segmentsToPieces(1, revenue, unchanged);
  assert.ok(isNoop(1, revenue, pieces));

  const edited = normalizeSegments(revenue, [textSeg("Revenue grew ", 0), textSeg("15", 1), textSeg(".5%", 1), textSeg(" this quarter", 2)]);
  assert.deepEqual(segmentsToPieces(1, revenue, edited), [
    { type: "keep", block: 1, inline: 0 },
    { type: "text", text: "15.5%", like: { block: 1, inline: 1 } },
    { type: "keep", block: 1, inline: 2 },
  ]);
});

test("text typed outside any run takes a neighbour's formatting", () => {
  const segs = normalizeSegments(revenue, [textSeg("New ", null), textSeg("Revenue grew ", 0)]);
  const pieces = segmentsToPieces(0, revenue, segs);
  assert.deepEqual(pieces[0], { type: "text", text: "New ", like: { block: 0, inline: 0 } });
  // In an empty paragraph there is nothing to copy.
  assert.deepEqual(segmentsToPieces(3, { inlines: [] }, normalizeSegments({ inlines: [] }, [textSeg("Hi", null)])), [
    { type: "text", text: "Hi" },
  ]);
});

test("visible objects can be deleted, invisible ones are always kept", () => {
  // The user deleted the picture.
  const noPicture = segmentsToPieces(2, picture, normalizeSegments(picture, [textSeg(" Figure 1", 1)]));
  assert.deepEqual(noPicture, [{ type: "keep", block: 2, inline: 1 }]);

  // The user selected "Page 1" and typed "X": the field's text run went,
  // but its invisible parts come back in order.
  const segs = normalizeSegments(field, [textSeg("X", 0)]);
  assert.deepEqual(
    segs.map((s) => [s.kind, s.inline]),
    [["text", 0], ["object", 1], ["object", 2], ["object", 3], ["object", 5], ["object", 6]],
  );
  const pieces = segmentsToPieces(7, field, segs);
  assert.deepEqual(pieces.map((p) => p.type), ["text", "keep", "keep", "keep", "keep", "keep"]);
});

test("normalizing merges, drops empties and ignores duplicates and unknown inlines", () => {
  const segs = normalizeSegments(link, [textSeg("Vi", 0), textSeg("", 0), textSeg("sit ", 0), objSeg(1), objSeg(1), objSeg(42), textSeg(".", 2)]);
  assert.deepEqual(segs, [textSeg("Visit ", 0), objSeg(1), textSeg(".", 2)]);
  // A text segment claiming to come from a hyperlink is treated as unformatted.
  assert.equal(normalizeSegments(link, [textSeg("x", 1)])[0].inline, null);
});

test("splitting segments never splits objects", () => {
  const segs = paragraphSegments(picture);
  assert.deepEqual(splitSegments(segs, 0).before, []);
  const { before, after } = splitSegments(segs, 4);
  assert.equal(segmentsText(before), OBJECT_CHAR + " Fi");
  assert.equal(segmentsText(after), "gure 1");
  assert.equal(after[0].inline, 1);
});

test("Enter splits a paragraph", () => {
  const segs = paragraphSegments(revenue);
  const at = { block: 1, paragraph: revenue, segments: segs, offset: "Revenue grew 1".length };
  const { splice, caret } = replaceRange(at, at, "\n");
  assert.deepEqual(splice, {
    first: 1,
    last: 1,
    content: [
      { type: "keep", block: 1, inline: 0 },
      { type: "text", text: "1", like: { block: 1, inline: 1 } },
      { type: "break" },
      { type: "text", text: "2%", like: { block: 1, inline: 1 } },
      { type: "keep", block: 1, inline: 2 },
    ],
  });
  assert.deepEqual(caret, { block: 2, offset: 0 });

  const end = { ...at, offset: segmentsLength(segs) };
  const atEnd = replaceRange(end, end, "\n").splice.content;
  assert.deepEqual(atEnd.slice(-1), [{ type: "break" }]);
  assert.equal(atEnd.filter((p) => p.type === "keep").length, 3, "the whole paragraph is kept");
});

test("Backspace at the start of a paragraph joins it to the previous one", () => {
  const prev = { block: 0, paragraph: { inlines: [run("Quarterly Report")] }, segments: paragraphSegments({ inlines: [run("Quarterly Report")] }) };
  const start = { ...prev, offset: segmentsLength(prev.segments) };
  const end = { block: 1, paragraph: revenue, segments: paragraphSegments(revenue), offset: 0 };
  const { splice, caret } = replaceRange(start, end, "");
  assert.deepEqual(splice, {
    first: 0,
    last: 1,
    content: [
      { type: "keep", block: 0, inline: 0 },
      { type: "keep", block: 1, inline: 0 },
      { type: "keep", block: 1, inline: 1 },
      { type: "keep", block: 1, inline: 2 },
    ],
  });
  assert.deepEqual(caret, { block: 0, offset: 16 });
});

test("replacing a selection across paragraphs keeps the edges and formats new text like the start", () => {
  const start = { block: 1, paragraph: revenue, segments: paragraphSegments(revenue), offset: "Revenue gr".length };
  const end = { block: 3, paragraph: link, segments: paragraphSegments(link), offset: "Visit ".length + 1 };
  const { splice, caret } = replaceRange(start, end, "owth");
  assert.deepEqual(splice, {
    first: 1,
    last: 3,
    content: [
      { type: "text", text: "Revenue gr", like: { block: 1, inline: 0 } },
      { type: "text", text: "owth", like: { block: 1, inline: 0 } },
      { type: "keep", block: 3, inline: 2 },
    ],
  });
  assert.deepEqual(caret, { block: 1, offset: "Revenue growth".length });
});

test("pasting several lines creates paragraphs and puts the caret at the end", () => {
  const at = { block: 0, paragraph: revenue, segments: paragraphSegments(revenue), offset: 0 };
  const { splice, caret } = replaceRange(at, at, "One\r\nTwo\nThree ");
  const types = splice.content.map((p) => (p.type === "text" ? p.text : p.type));
  assert.deepEqual(types, ["One", "break", "Two", "break", "Three ", "keep", "keep", "keep"]);
  assert.deepEqual(caret, { block: 2, offset: "Three ".length });
  // Pasted text at the very start takes the formatting of what follows.
  assert.deepEqual(splice.content[0].like, { block: 0, inline: 0 });
});

test("deleting a range that contains invisible field parts keeps them", () => {
  const segs = paragraphSegments(field);
  const start = { block: 7, paragraph: field, segments: segs, offset: 2 }; // "Pa|ge "
  const end = { block: 7, paragraph: field, segments: segs, offset: segmentsLength(segs) - 1 }; // before the comment ref
  const { splice } = replaceRange(start, end, "");
  const kept = splice.content.filter((p) => p.type === "keep").map((p) => p.inline);
  assert.deepEqual(kept, [1, 2, 3, 5, 6], "field codes and the comment reference survive");
  assert.deepEqual(splice.content[0], { type: "text", text: "Pa", like: { block: 7, inline: 0 } });
});

test("remapping follows kept inlines and the formatting of new runs", () => {
  const map = remapAfterSplice([
    { type: "keep", block: 1, inline: 0 },
    { type: "text", text: "15.5%", like: { block: 1, inline: 1 } },
    { type: "text", text: "", like: { block: 1, inline: 9 } },
    { type: "keep", block: 1, inline: 2 },
  ]);
  assert.deepEqual([...map.entries()], [[0, 0], [1, 1], [2, 2]]);

  const shifted = remapAfterSplice([{ type: "text", text: "A", like: { block: 0, inline: 3 } }, { type: "keep", block: 0, inline: 3 }]);
  assert.equal(shifted.get(3), 1, "a kept run maps to itself, not to text formatted like it");
});
