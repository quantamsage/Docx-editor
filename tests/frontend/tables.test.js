import { test } from "node:test";
import assert from "node:assert/strict";

import { containerKey } from "../../web/js/editor/containers.js";
import { hasBorders, tableEdits, tableSize } from "../../web/js/editor/tables.js";

const cells = (n) => Array.from({ length: n }, () => ({ span: 1, blocks: [] }));
const table = { type: "table", grid: [3000, 3000, 3000], rows: [{ cells: cells(3) }, { cells: cells(3) }] };
const at = { container: "", table: 4, row: 1, cell: 2 };

test("row and column commands insert next to the cursor's cell and move the cursor there", () => {
  assert.deepEqual(tableEdits("row-above", at, table), { edits: [{ op: "insert_row", table: 4, row: 1, after: false }], caret: { row: 1, cell: 2 } });
  assert.deepEqual(tableEdits("row-below", at, table).caret, { row: 2, cell: 2 });
  assert.deepEqual(tableEdits("column-left", at, table).edits, [{ op: "insert_column", table: 4, row: 1, cell: 2, after: false }]);
  assert.deepEqual(tableEdits("column-right", at, table).caret, { row: 1, cell: 3 });
  assert.deepEqual(tableEdits("delete-row", at, table), { edits: [{ op: "delete_row", table: 4, row: 1 }], caret: { row: 0, cell: 2 } });
  assert.deepEqual(tableEdits("delete-column", at, table), { edits: [{ op: "delete_column", table: 4, row: 1, cell: 2 }], caret: { row: 1, cell: 1 } });
  assert.deepEqual(tableEdits("delete-table", at, table), { edits: [{ op: "delete_block", block: 4 }], caret: null });
});

test("tables keep at least one row and column", () => {
  const single = { type: "table", grid: [9000], rows: [{ cells: cells(1) }] };
  const corner = { container: "", table: 0, row: 0, cell: 0 };
  assert.match(tableEdits("delete-row", corner, single).error, /at least one row/);
  assert.match(tableEdits("delete-column", corner, single).error, /at least one column/);
  assert.match(tableEdits("explode", corner, single).error, /Unknown/);
});

test("appearance commands and tables in headers", () => {
  const inHeader = { ...at, container: containerKey({ part: "word/header1.xml" }), table: 0 };
  assert.deepEqual(tableEdits("borders", inHeader, table, false).edits, [
    { op: "set_table_borders", container: { part: "word/header1.xml" }, table: 0, borders: false },
  ]);
  assert.deepEqual(tableEdits("cell-align", at, table, "center").edits[0].format, { vertical_align: "center" });
  assert.deepEqual(tableEdits("cell-shading", at, table, "#fff2cc").edits[0].format, { shading: "FFF2CC" });
  assert.deepEqual(tableEdits("cell-shading", at, table, null).edits[0].format, { shading: null });
  assert.match(tableEdits("delete-table", inHeader, table).error, /headers and footers/);

  assert.equal(hasBorders({ borders: false, style_id: "TableGrid" }), false);
  assert.equal(hasBorders({ style_id: "TableGrid" }), true);
  assert.equal(hasBorders({}), false);
});

test("table size from the dialog", () => {
  assert.deepEqual(tableSize("2", "3"), { rows: 2, columns: 3 });
  assert.equal(tableSize("0", "3"), null);
  assert.equal(tableSize("2", "64"), null);
  assert.equal(tableSize("", "3"), null);
});
