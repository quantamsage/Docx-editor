// Insert and layout tools: the Insert group of the main toolbar, its
// dialogs, and the contextual table toolbar shown while the cursor is in a
// table. The edits themselves are built by the editor and tables.js.

import { TABLE_COMMANDS, hasBorders, tableEdits, tableSize } from "./tables.js";

/**
 * @param {object} options
 * @param {ReturnType<import("./editor.js").createEditor>} options.editor
 * @param {HTMLElement} options.insertBar  Holds the [data-insert] buttons.
 * @param {HTMLElement} options.tableBar  The table toolbar ([data-table] controls).
 * @param {(message: string) => void} options.onInfo
 */
export function createInsertTools({ editor, insertBar, tableBar, onInfo }) {
  const $ = (id) => document.getElementById(id);
  let state = null;

  // ----- Keeping the editor's selection across dialogs -----

  function saveSelection() {
    const selection = document.getSelection();
    if (!selection || !selection.rangeCount) return null;
    const range = selection.getRangeAt(0);
    return editor.hosts.some((h) => h.contains(range.startContainer)) ? range.cloneRange() : null;
  }

  function restoreSelection(range) {
    if (!range || !range.startContainer.isConnected) return;
    const host = editor.hosts.find((h) => h.contains(range.startContainer));
    if (host) host.focus({ preventScroll: true });
    const selection = document.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
  }

  /** Shows a modal dialog; resolves with true when it was confirmed. */
  function ask(dialog, prepare = () => {}) {
    const range = saveSelection();
    prepare();
    return new Promise((resolve) => {
      dialog.addEventListener(
        "close",
        () => {
          restoreSelection(range);
          resolve(dialog.returnValue === "ok");
        },
        { once: true },
      );
      dialog.returnValue = "";
      dialog.showModal();
    });
  }

  // ----- Insert group -----

  async function insertTable() {
    if (!(await ask($("table-dialog"), () => $("table-columns").select()))) return;
    const size = tableSize($("table-rows").value, $("table-columns").value);
    if (!size) {
      onInfo("Tables can have 1 to 500 rows and 1 to 63 columns.");
      return;
    }
    await editor.insertTable(size.rows, size.columns);
  }

  const actions = { table: insertTable };

  insertBar.addEventListener("mousedown", (event) => {
    if (event.target.closest("button[data-insert]")) event.preventDefault();
  });
  insertBar.addEventListener("click", (event) => {
    const button = event.target.closest("button[data-insert]");
    if (!button || button.disabled || !editor.document) return;
    const action = actions[button.dataset.insert];
    if (action) action(button);
  });

  // ----- Table toolbar -----

  function tableCommand(command, value) {
    if (!TABLE_COMMANDS.includes(command)) return Promise.resolve();
    return editor.tableCommand((cell, table) => tableEdits(command, cell, table, value)).then(update);
  }

  tableBar.addEventListener("mousedown", (event) => {
    if (event.target.closest("button[data-table]")) event.preventDefault();
  });
  tableBar.addEventListener("click", (event) => {
    const button = event.target.closest("button[data-table]");
    if (!button) return;
    const command = button.dataset.table;
    if (command === "borders") tableCommand(command, button.getAttribute("aria-pressed") !== "true");
    else if (command === "cell-shading") tableCommand(command, button.dataset.value || null);
    else tableCommand(command);
  });
  tableBar.addEventListener("change", (event) => {
    const control = event.target.closest("[data-table]");
    if (control && control.tagName !== "BUTTON") tableCommand(control.dataset.table, control.value);
  });

  /** Reflects the cursor's position: enables tools and shows the table toolbar in tables. */
  function update() {
    state = editor.document ? editor.currentFormat() : null;
    for (const button of insertBar.querySelectorAll("[data-insert]")) button.disabled = !editor.document;
    const cell = state && state.cell && state.table && state.table.type === "table" ? state.table.rows[state.cell.row]?.cells[state.cell.cell] : null;
    tableBar.hidden = !cell;
    if (!cell) return;
    tableBar.querySelector('[data-table="borders"]').setAttribute("aria-pressed", String(hasBorders(state.table)));
    $("cell-align").value = cell.vertical_align || "top";
    if (cell.shading) $("cell-shading").value = `#${cell.shading.toLowerCase()}`;
    tableBar.querySelector('[data-table="delete-table"]').disabled = Boolean(state.cell.container);
  }

  return { update, tableCommand, insertTable };
}
