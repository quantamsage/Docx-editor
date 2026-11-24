// Table commands: from the cell at the cursor to the engine's table edits
// and the cell the cursor goes to afterwards. No DOM access, so it is
// unit-tested in Node.

import { containerField } from "./containers.js";

/** Table commands offered by the table toolbar. */
export const TABLE_COMMANDS = [
  "row-above",
  "row-below",
  "column-left",
  "column-right",
  "delete-row",
  "delete-column",
  "delete-table",
  "borders",
  "cell-align",
  "cell-shading",
];

/** Largest table the Insert table dialog accepts. */
export const MAX_TABLE = { rows: 500, columns: 63 };

/**
 * Edits for a table command.
 *
 * @param {string} command  One of TABLE_COMMANDS.
 * @param {{container: string, table: number, row: number, cell: number}} at  The cell at the cursor
 *   (`container` is the key of the container holding the table).
 * @param {object} table  The table from the model.
 * @param {unknown} [value]  For "borders": true/false; "cell-align": top/center/bottom;
 *   "cell-shading": a hex colour without "#" or null.
 * @returns {{edits: object[], caret: {row: number, cell: number}}|{error: string}}
 */
export function tableEdits(command, at, table, value) {
  const where = { ...containerField(at.container), table: at.table };
  const rows = (table && table.rows) || [];
  const cells = rows[at.row] ? rows[at.row].cells.length : 0;
  switch (command) {
    case "row-above":
      return { edits: [{ op: "insert_row", ...where, row: at.row, after: false }], caret: { row: at.row, cell: at.cell } };
    case "row-below":
      return { edits: [{ op: "insert_row", ...where, row: at.row, after: true }], caret: { row: at.row + 1, cell: at.cell } };
    case "column-left":
      return { edits: [{ op: "insert_column", ...where, row: at.row, cell: at.cell, after: false }], caret: { row: at.row, cell: at.cell } };
    case "column-right":
      return { edits: [{ op: "insert_column", ...where, row: at.row, cell: at.cell, after: true }], caret: { row: at.row, cell: at.cell + 1 } };
    case "delete-row":
      if (rows.length <= 1) return { error: "A table needs at least one row. Use Delete table instead." };
      return { edits: [{ op: "delete_row", ...where, row: at.row }], caret: { row: Math.min(at.row, rows.length - 2), cell: at.cell } };
    case "delete-column":
      if (rows.every((r) => r.cells.length <= 1) && !(table.grid && table.grid.length > 1)) {
        return { error: "A table needs at least one column. Use Delete table instead." };
      }
      return { edits: [{ op: "delete_column", ...where, row: at.row, cell: at.cell }], caret: { row: at.row, cell: Math.max(0, Math.min(at.cell, cells - 2)) } };
    case "delete-table":
      if (at.container) return { error: "Tables in headers and footers can't be deleted here." };
      return { edits: [{ op: "delete_block", block: at.table }], caret: null };
    case "borders":
      return { edits: [{ op: "set_table_borders", ...where, borders: Boolean(value) }], caret: { row: at.row, cell: at.cell } };
    case "cell-align":
      return {
        edits: [{ op: "format_cell", ...where, row: at.row, cell: at.cell, format: { vertical_align: value || null } }],
        caret: { row: at.row, cell: at.cell },
      };
    case "cell-shading":
      return {
        edits: [{ op: "format_cell", ...where, row: at.row, cell: at.cell, format: { shading: value ? String(value).replace("#", "").toUpperCase() : null } }],
        caret: { row: at.row, cell: at.cell },
      };
    default:
      return { error: `Unknown table command ${command}.` };
  }
}

/** Whether a table shows borders (set on it, or from its style). */
export function hasBorders(table) {
  if (!table) return false;
  return table.borders === true || (table.borders === undefined && Boolean(table.style_id));
}

/** Parses the Insert table dialog's values; null when out of range. */
export function tableSize(rows, columns) {
  const r = Number.parseInt(rows, 10);
  const c = Number.parseInt(columns, 10);
  if (!(r >= 1 && r <= MAX_TABLE.rows && c >= 1 && c <= MAX_TABLE.columns)) return null;
  return { rows: r, columns: c };
}

/**
 * The cell Tab (`step` 1) or Shift+Tab (-1) moves to from `rows[row].cells[cell]`,
 * in reading order, skipping cells that continue a vertical merge (they
 * are drawn as part of the cell above); null at the end of the table.
 */
export function nextCell(table, row, cell, step) {
  const rows = (table && table.rows) || [];
  let r = row;
  let c = cell;
  for (;;) {
    c += step;
    if (c < 0) {
      r -= 1;
      if (r < 0) return null;
      c = rows[r].cells.length - 1;
    } else if (!rows[r] || c >= rows[r].cells.length) {
      r += 1;
      c = 0;
      if (r >= rows.length) return null;
    }
    if (!rows[r].cells.length) continue;
    if (rows[r].cells[c].vertical_merge !== "continue") return { row: r, cell: c };
  }
}
