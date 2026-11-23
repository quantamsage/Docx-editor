// Insert and layout tools: the Insert group of the main toolbar, its
// dialogs, and the contextual table toolbar shown while the cursor is in a
// table. The edits themselves are built by the editor and tables.js.

import { uploadImage } from "../api.js";
import { MARGIN_PRESETS, PAPER_SIZES, marginPresetId, marginsOf, paperId, pictureSize, sectionFormat, twipsToCm } from "../layout.js";
import { containerKey } from "./containers.js";
import { TABLE_COMMANDS, hasBorders, tableEdits, tableSize } from "./tables.js";

/** Picture types Word and browsers both display. */
export const PICTURE_TYPES = ["image/png", "image/jpeg", "image/gif", "image/bmp"];
/** Largest picture accepted (as on the server). */
export const MAX_PICTURE_BYTES = 25 * 1024 * 1024;

/**
 * @param {object} options
 * @param {ReturnType<import("./editor.js").createEditor>} options.editor
 * @param {HTMLElement} options.insertBar  Holds the [data-insert] buttons.
 * @param {HTMLElement} options.tableBar  The table toolbar ([data-table] controls).
 * @param {(message: string) => void} options.onInfo
 * @param {(message: string) => void} [options.onError]
 */
export function createInsertTools({ editor, insertBar, tableBar, onInfo, onError = onInfo }) {
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

  /** Uploads a picture file and inserts it at the cursor, fitted to the text width. */
  async function insertPicture(file, range = saveSelection()) {
    if (!editor.document || !file) return;
    if (!PICTURE_TYPES.includes(file.type) && !/\.(png|jpe?g|gif|bmp)$/i.test(file.name || "")) {
      onInfo(`"${file.name}" can't be inserted. Use a PNG, JPEG, GIF or BMP picture.`);
      return;
    }
    if (file.size > MAX_PICTURE_BYTES) {
      onInfo("The picture is larger than 25 MiB.");
      return;
    }
    let info;
    try {
      info = await uploadImage(editor.document.id, file);
    } catch (err) {
      onError(err.message || String(err));
      return;
    }
    restoreSelection(range);
    const size = pictureSize(info.width_px, info.height_px, editor.document.model.section);
    const description = (file.name || "").replace(/\.[^.]+$/, "");
    await editor.insertImage({ image: info.id, ...size, description });
  }

  function choosePicture() {
    const range = saveSelection();
    const input = $("picture-input");
    input.value = "";
    input.onchange = () => {
      if (input.files && input.files[0]) insertPicture(input.files[0], range);
    };
    input.click();
  }

  /**
   * Puts the cursor at the end of the header or footer, creating an empty
   * one first if the document has none.
   */
  async function editHeaderFooter(kind) {
    const has = () => editor.document && editor.document.model[kind];
    if (!has()) await editor.applyDocumentEdits([{ op: "add_header_footer", kind }]);
    const hf = has();
    if (!hf) return;
    const blocks = hf.blocks || [];
    const last = Math.max(0, blocks.map((b) => b.type).lastIndexOf("paragraph"));
    editor.placeCaret(last, Number.MAX_SAFE_INTEGER, containerKey({ part: hf.part }));
  }

  async function insertPageNumbers() {
    if (!(await ask($("page-numbers-dialog")))) return;
    const kind = $("page-numbers-position").value === "header" ? "header" : "footer";
    const hf = editor.document.model[kind];
    if (hf && hasPageNumbers(hf.blocks)) {
      onInfo(`The ${kind} already has page numbers.`);
      return;
    }
    const edits = [
      {
        op: "insert_page_numbers",
        kind,
        alignment: $("page-numbers-alignment").value,
        with_total: $("page-numbers-total").checked,
      },
    ];
    const start = $("page-numbers-start").value.trim();
    if (start !== "") {
      const n = Number.parseInt(start, 10);
      if (!(n >= 0 && n <= 32767)) {
        onInfo("Page numbers can start at 0 to 32767.");
        return;
      }
      edits.push({ op: "format_section", format: { page_number_start: n } });
    }
    await editor.applyDocumentEdits(edits);
  }

  const SIDES = ["top", "bottom", "left", "right"];

  function fillPageSetup() {
    const section = editor.document.model.section || {};
    const paper = paperId(section);
    const options = PAPER_SIZES.map((p) => new Option(p.label, p.id));
    if (paper === "custom") options.push(new Option("Current size (custom)", "custom"));
    $("page-paper").replaceChildren(...options);
    $("page-paper").value = paper;
    const landscape = Boolean(section.landscape) || (section.page_width || 0) > (section.page_height || 0);
    $(landscape ? "page-landscape" : "page-portrait").checked = true;
    const presets = MARGIN_PRESETS.map((p) => new Option(p.label, p.id));
    presets.push(new Option("Custom", "custom"));
    $("page-margin-preset").replaceChildren(...presets);
    $("page-margin-preset").value = marginPresetId(section);
    const margins = marginsOf(section);
    for (const side of SIDES) $(`page-margin-${side}`).value = String(twipsToCm(margins[side]));
  }

  $("page-margin-preset").addEventListener("change", () => {
    const preset = MARGIN_PRESETS.find((p) => p.id === $("page-margin-preset").value);
    if (preset) for (const side of SIDES) $(`page-margin-${side}`).value = String(twipsToCm(preset[side]));
  });
  for (const side of SIDES) {
    $(`page-margin-${side}`).addEventListener("input", () => {
      $("page-margin-preset").value = "custom";
    });
  }

  async function pageSetup() {
    if (!(await ask($("page-setup-dialog"), fillPageSetup))) return;
    const margins = Object.fromEntries(SIDES.map((side) => [side, $(`page-margin-${side}`).value]));
    const result = sectionFormat({
      paper: $("page-paper").value,
      landscape: $("page-landscape").checked,
      margins,
      section: editor.document.model.section,
    });
    if (result.error) {
      onInfo(result.error);
      return;
    }
    await editor.applyDocumentEdits([{ op: "format_section", format: result.format }]);
  }

  const actions = {
    "page-break": () => editor.insertPageBreak(),
    "page-setup": pageSetup,
    table: insertTable,
    picture: choosePicture,
    header: () => editHeaderFooter("header"),
    footer: () => editHeaderFooter("footer"),
    "page-numbers": insertPageNumbers,
  };

  // Double-clicking the empty top or bottom margin adds a header or footer.
  for (const [kind, host] of [["header", editor.hosts[1]], ["footer", editor.hosts[2]]]) {
    if (!host) continue;
    host.addEventListener("dblclick", () => {
      if (editor.document && !editor.document.model[kind]) editHeaderFooter(kind);
    });
  }

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

  return { update, tableCommand, insertTable, insertPicture, editHeaderFooter };
}

/** Whether header or footer blocks already show a page number (a PAGE field). */
export function hasPageNumbers(blocks) {
  return (blocks || []).some(
    (b) => b.type === "paragraph" && (b.inlines || []).some((i) => i.type === "other" && i.field === "PAGE"),
  );
}
