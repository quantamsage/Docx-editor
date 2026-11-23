// Selecting and resizing pictures on the page.
//
// Clicking a picture selects it (so Delete or Backspace removes it, like
// any object) and shows a handle at its bottom-right corner. Dragging the
// handle resizes the picture in proportion; when it is released the new
// size is sent to the document.

import { EMU_PER_PIXEL, resizedPicture } from "../layout.js";

/**
 * @param {ReturnType<import("./editor.js").createEditor>} editor
 */
export function enablePictureResizing(editor) {
  let selected = null;

  function deselect() {
    if (!selected) return;
    selected.classList.remove("is-selected");
    selected.querySelector(".resize-handle")?.remove();
    selected = null;
  }

  function select(object) {
    if (selected === object) return;
    deselect();
    selected = object;
    object.classList.add("is-selected");
    const handle = document.createElement("span");
    handle.className = "resize-handle";
    handle.title = "Drag to resize the picture";
    handle.setAttribute("aria-hidden", "true");
    object.appendChild(handle);
    // Select the picture as one object, so that typing or Delete replaces it.
    const range = document.createRange();
    range.selectNode(object);
    const selection = document.getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
  }

  /** Where a picture is: its paragraph's container and block, and its inline index. */
  function locate(object) {
    const paragraph = object.closest("[data-block][data-kind='paragraph']");
    const inline = Number(object.dataset.i);
    if (!paragraph || !Number.isInteger(inline) || inline < 0) return null;
    return { key: paragraph.dataset.container || "", block: Number(paragraph.dataset.block), inline };
  }

  function startResize(event, object) {
    const img = object.querySelector("img");
    const where = locate(object);
    if (!img || !where) return;
    event.preventDefault();
    event.stopPropagation();
    const startX = event.clientX;
    const startWidth = img.getBoundingClientRect().width;
    const startHeight = img.getBoundingClientRect().height;
    const host = object.closest("[contenteditable=true]");
    const maxWidth = host ? host.clientWidth : Infinity;
    let size = { width: Math.round(startWidth), height: Math.round(startHeight) };
    const handle = event.target;
    handle.setPointerCapture(event.pointerId);
    const move = (e) => {
      size = resizedPicture(startWidth, startHeight, e.clientX - startX, maxWidth);
      img.width = size.width;
      img.height = size.height;
    };
    const end = () => {
      handle.removeEventListener("pointermove", move);
      handle.removeEventListener("pointerup", end);
      handle.removeEventListener("pointercancel", end);
      if (Math.abs(size.width - Math.round(startWidth)) < 1) return;
      deselect();
      editor.resizeImage(where.key, where.block, where.inline, size.width * EMU_PER_PIXEL, size.height * EMU_PER_PIXEL);
    };
    handle.addEventListener("pointermove", move);
    handle.addEventListener("pointerup", end);
    handle.addEventListener("pointercancel", end);
  }

  for (const host of editor.hosts) {
    host.addEventListener("pointerdown", (event) => {
      if (event.target.classList?.contains("resize-handle")) {
        startResize(event, event.target.closest(".obj-image"));
        return;
      }
      const object = event.target.closest?.(".obj-image");
      if (object && object.querySelector("img")) {
        event.preventDefault();
        select(object);
      } else {
        deselect();
      }
    });
    host.addEventListener("keydown", () => deselect());
  }
  document.addEventListener("selectionchange", () => {
    if (!selected) return;
    const selection = document.getSelection();
    const range = selection.rangeCount ? selection.getRangeAt(0) : null;
    if (!range || !range.intersectsNode(selected) || !selected.isConnected) deselect();
  });

  return { deselect };
}
