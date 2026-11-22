import { test } from "node:test";
import assert from "node:assert/strict";

import { shortcutFor, steppedIndent, steppedSize, styleLabel, styleOptions } from "../../web/js/editor/toolbar.js";

test("style menu: Normal, built-in headings, then the document's paragraph styles", () => {
  const options = styleOptions([
    { id: "Normal", name: "Normal", kind: "paragraph" },
    { id: "Heading1", name: "heading 1", kind: "paragraph" },
    { id: "Quote", name: "Quote", kind: "paragraph" },
    { id: "Hyperlink", name: "Hyperlink", kind: "character" },
    { id: "Caption", name: "caption", kind: "paragraph" },
  ]);
  assert.deepEqual(options.map((o) => o.id), ["", "Title", "Heading1", "Heading2", "Heading3", "Caption", "Quote"]);
  assert.equal(options[2].label, "Heading 1");
  assert.equal(styleLabel({ id: "x", name: "caption" }), "Caption");
});

test("indent and size steps", () => {
  assert.equal(steppedIndent(undefined, 1), 720);
  assert.equal(steppedIndent(720, 1), 1440);
  assert.equal(steppedIndent(1000, -1), 720, "snaps to half inches");
  assert.equal(steppedIndent(300, -1), 0, "never negative");
  assert.equal(steppedSize(22, 1), 24, "11 → 12 pt");
  assert.equal(steppedSize(22, -1), 21, "11 → 10.5 pt");
  assert.equal(steppedSize(undefined, 1), 24, "default 11 pt");
  assert.equal(steppedSize(144, 1), 144, "72 pt is the largest");
});

test("keyboard shortcuts", () => {
  const key = (k, extra = {}) => ({ key: k, code: extra.code || "", ctrlKey: true, metaKey: false, shiftKey: false, altKey: false, ...extra });
  assert.equal(shortcutFor(key("b")), "bold");
  assert.equal(shortcutFor(key("B")), "bold");
  assert.equal(shortcutFor(key("]")), "grow");
  assert.equal(shortcutFor(key("E", { shiftKey: true })), "align-center");
  assert.equal(shortcutFor(key("*", { shiftKey: true, code: "Digit8" })), "bullets");
  assert.equal(shortcutFor(key("&", { shiftKey: true, code: "Digit7" })), "numbering");
  assert.equal(shortcutFor(key("2", { altKey: true })), "heading2");
  assert.equal(shortcutFor(key("0", { altKey: true })), "normal");
  assert.equal(shortcutFor(key("s")), null, "Ctrl+S is the app's Save");
  assert.equal(shortcutFor({ ...key("b"), ctrlKey: false }), null);
});
