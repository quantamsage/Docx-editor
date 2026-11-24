import { test } from "node:test";
import assert from "node:assert/strict";

import { toolbarIndex } from "../../web/js/keyboard.js";

test("arrow keys, Home and End move through a toolbar and wrap around", () => {
  assert.equal(toolbarIndex(5, 0, "ArrowRight"), 1);
  assert.equal(toolbarIndex(5, 4, "ArrowRight"), 0);
  assert.equal(toolbarIndex(5, 0, "ArrowLeft"), 4);
  assert.equal(toolbarIndex(5, 2, "ArrowDown"), 3);
  assert.equal(toolbarIndex(5, 2, "ArrowUp"), 1);
  assert.equal(toolbarIndex(5, 3, "Home"), 0);
  assert.equal(toolbarIndex(5, 1, "End"), 4);
  assert.equal(toolbarIndex(5, 1, "a"), null);
  assert.equal(toolbarIndex(0, 0, "ArrowRight"), null);
});
