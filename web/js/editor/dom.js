// Reading the editing surface: paragraph content, positions and selection.
//
// Positions are character offsets within a paragraph, counted the same way
// as in content.js: text characters, line breaks (<br>) and objects each
// count as one; the placeholder <br> of an empty paragraph counts as zero.

const TEXT_NODE = 3;
const ELEMENT_NODE = 1;

function isObject(node) {
  return node.nodeType === ELEMENT_NODE && node.dataset && node.dataset.kind === "object";
}

function isPlaceholder(node) {
  return node.nodeName === "BR" && node.dataset && "placeholder" in node.dataset;
}

/** The top-level block element containing `node`, or null. */
export function blockElement(root, node) {
  let n = node && node.nodeType === TEXT_NODE ? node.parentNode : node;
  while (n && n !== root) {
    if (n.parentNode === root && n.dataset && n.dataset.block !== undefined) return n;
    n = n.parentNode;
  }
  return null;
}

/**
 * Reads a paragraph element into segments: `{kind: "text", text, inline}`
 * (inline = the run index of the enclosing run span, or null for text the
 * browser put outside any run) and `{kind: "object", inline}`.
 */
export function parseParagraph(el) {
  const out = [];
  const walk = (node, inline) => {
    for (const child of node.childNodes) {
      if (child.nodeType === TEXT_NODE) {
        if (child.data) out.push({ kind: "text", text: child.data.replace(/​/g, ""), inline });
      } else if (child.nodeType === ELEMENT_NODE) {
        if (isObject(child)) {
          const i = Number(child.dataset.i);
          if (Number.isInteger(i) && i >= 0) out.push({ kind: "object", inline: i });
        } else if (child.nodeName === "BR") {
          if (!isPlaceholder(child)) out.push({ kind: "text", text: "\n", inline });
        } else {
          const own = child.dataset && child.dataset.kind === "run" && child.dataset.i !== "" ? Number(child.dataset.i) : inline;
          walk(child, Number.isInteger(own) ? own : inline);
        }
      }
    }
  };
  walk(el, null);
  return out;
}

/** Character offset of the DOM position (node, offset) within paragraph `el`. */
export function offsetOf(el, node, offset) {
  let count = 0;
  let found = null;
  const walk = (n) => {
    if (found !== null) return;
    if (n.nodeType === TEXT_NODE) {
      if (n === node) found = count + Math.min(offset, n.data.length);
      else count += n.data.length;
      return;
    }
    if (n !== el && isObject(n)) {
      if (n === node || n.contains(node)) {
        found = count + (n === node && offset > 0 ? 1 : 0);
        return;
      }
      count += 1;
      return;
    }
    if (n.nodeName === "BR") {
      if (n === node) found = count;
      else if (!isPlaceholder(n)) count += 1;
      return;
    }
    const kids = n.childNodes;
    for (let i = 0; i < kids.length; i++) {
      if (n === node && i === offset) {
        found = count;
        return;
      }
      walk(kids[i]);
      if (found !== null) return;
    }
    if (n === node) found = count;
  };
  walk(el);
  return found === null ? count : found;
}

/** Total length of a paragraph element in characters. */
export function paragraphLength(el) {
  return offsetOf(el, el, el.childNodes.length);
}

/** The DOM position for a character offset within paragraph `el`. */
export function positionAt(el, target) {
  let count = 0;
  let result = null;
  const walk = (n) => {
    for (let i = 0; i < n.childNodes.length && !result; i++) {
      const child = n.childNodes[i];
      if (child.nodeType === TEXT_NODE) {
        const len = child.data.length;
        if (target <= count + len) {
          result = { node: child, offset: target - count };
          return;
        }
        count += len;
      } else if (isObject(child) || (child.nodeName === "BR" && !isPlaceholder(child))) {
        if (target === count) {
          result = { node: n, offset: i };
          return;
        }
        count += 1;
      } else if (child.nodeName === "BR") {
        if (target <= count) {
          result = { node: n, offset: i };
          return;
        }
      } else if (child.nodeType === ELEMENT_NODE) {
        walk(child);
      }
    }
  };
  walk(el);
  return result || { node: el, offset: el.childNodes.length - (el.lastChild && isPlaceholder(el.lastChild) ? 1 : 0) };
}

/** Places a collapsed caret at a character offset in paragraph `el`. */
export function setCaret(el, offset) {
  const { node, offset: at } = positionAt(el, Math.max(0, offset));
  const selection = el.ownerDocument.getSelection();
  const range = el.ownerDocument.createRange();
  range.setStart(node, at);
  range.collapse(true);
  selection.removeAllRanges();
  selection.addRange(range);
}

function endpoint(root, container, offset) {
  let block;
  let at;
  if (container === root) {
    const blocks = root.children;
    if (!blocks.length) return null;
    block = blocks[Math.min(offset, blocks.length - 1)];
    at = offset >= blocks.length ? "end" : 0;
  } else {
    block = blockElement(root, container);
    if (!block) return null;
  }
  const index = Number(block.dataset.block);
  if (block.dataset.kind !== "paragraph") return { block: index, el: block, offset: 0, editable: false };
  const off = at === "end" ? paragraphLength(block) : at === 0 ? 0 : offsetOf(block, container, offset);
  return { block: index, el: block, offset: off, editable: true };
}

/**
 * The current selection within `root` as block/offset positions:
 * `{start, end, collapsed}`, or null when the selection is elsewhere.
 * Positions inside tables or other non-editable blocks are marked
 * `editable: false`.
 */
export function selectionInfo(root) {
  const selection = root.ownerDocument.getSelection();
  if (!selection || !selection.rangeCount) return null;
  const range = selection.getRangeAt(0);
  if (!root.contains(range.startContainer) || !root.contains(range.endContainer)) return null;
  const start = endpoint(root, range.startContainer, range.startOffset);
  const end = endpoint(root, range.endContainer, range.endOffset);
  if (!start || !end) return null;
  return { start, end, collapsed: range.collapsed };
}

/** Inserts plain text at the selection (used for Tab). */
export function insertTextAtSelection(doc, text) {
  const selection = doc.getSelection();
  if (!selection.rangeCount) return;
  const range = selection.getRangeAt(0);
  range.deleteContents();
  const node = doc.createTextNode(text);
  range.insertNode(node);
  range.setStartAfter(node);
  range.collapse(true);
  selection.removeAllRanges();
  selection.addRange(range);
}
