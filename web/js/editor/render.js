// Renders the document model as an editable page.
//
// Every top-level block gets `data-block` (its index in the model) and
// `data-kind`. Paragraphs are editable; tables and preserved blocks are
// shown read-only. Inside a paragraph, text runs are `<span data-kind="run"
// data-i="…">` and everything else is a non-editable
// `<span data-kind="object" data-i="…">`. Formatting is applied through
// the CSSOM (element.style), which the Content-Security-Policy allows.

import { twipsToPx } from "../documents.js";
import { paragraphSegments } from "./content.js";

const HIGHLIGHTS = {
  yellow: "#ffff00",
  green: "#00ff00",
  cyan: "#00ffff",
  magenta: "#ff00ff",
  blue: "#0000ff",
  red: "#ff0000",
  darkBlue: "#000080",
  darkCyan: "#008080",
  darkGreen: "#008000",
  darkMagenta: "#800080",
  darkRed: "#800000",
  darkYellow: "#808000",
  darkGray: "#808080",
  lightGray: "#c0c0c0",
  black: "#000000",
  white: "#ffffff",
};

/** CSS class for a paragraph style, from its name ("heading 1" → para-heading-1). */
export function styleClass(styleId, styles) {
  if (!styleId) return null;
  const style = (styles || []).find((s) => s.id === styleId);
  const name = (style ? style.name : styleId).toLowerCase().replace(/\s+/g, " ").trim();
  const heading = /^heading ?([1-6])$/.exec(name);
  if (heading) return `para-heading-${heading[1]}`;
  if (name === "title") return "para-title";
  if (name === "subtitle") return "para-subtitle";
  if (name === "quote" || name === "intense quote") return "para-quote";
  return null;
}

/** Applies run formatting to a span. */
export function applyRunStyle(el, props) {
  if (!props) return;
  const s = el.style;
  if (props.bold) s.fontWeight = "bold";
  if (props.italic) s.fontStyle = "italic";
  const decorations = [];
  if (props.underline) decorations.push("underline");
  if (props.strike) decorations.push("line-through");
  if (decorations.length) s.textDecoration = decorations.join(" ");
  if (props.font) s.fontFamily = `"${props.font.replace(/["\\]/g, "")}", var(--doc-font)`;
  if (props.size) s.fontSize = `${props.size / 2}pt`;
  if (props.color && /^[0-9a-f]{6}$/i.test(props.color)) s.color = `#${props.color}`;
  if (props.highlight && HIGHLIGHTS[props.highlight]) s.backgroundColor = HIGHLIGHTS[props.highlight];
  if (props.vertical_align === "superscript" || props.vertical_align === "subscript") {
    s.verticalAlign = props.vertical_align === "superscript" ? "super" : "sub";
    s.fontSize = "0.75em";
  }
}

/** Applies paragraph properties (alignment, indentation, spacing). */
export function applyParagraphStyle(el, paragraph) {
  const s = el.style;
  if (paragraph.alignment) s.textAlign = paragraph.alignment;
  const ind = paragraph.indent;
  if (ind) {
    if (ind.left) s.marginLeft = `${twipsToPx(ind.left)}px`;
    if (ind.right) s.marginRight = `${twipsToPx(ind.right)}px`;
    if (ind.first_line) s.textIndent = `${twipsToPx(ind.first_line)}px`;
  }
  const sp = paragraph.spacing;
  if (sp) {
    if (sp.before !== undefined && sp.before !== null) s.marginTop = `${twipsToPx(sp.before)}px`;
    if (sp.after !== undefined && sp.after !== null) s.marginBottom = `${twipsToPx(sp.after)}px`;
    if (sp.line) {
      s.lineHeight = !sp.line_rule || sp.line_rule === "auto" ? String((sp.line / 240) * 1.15) : `${twipsToPx(sp.line)}px`;
    }
  }
}

function appendText(parent, text) {
  // Line breaks are <br>; tabs and spaces stay as text (white-space: pre-wrap).
  const parts = text.split("\n");
  parts.forEach((part, i) => {
    if (i > 0) parent.appendChild(document.createElement("br"));
    if (part) parent.appendChild(document.createTextNode(part));
  });
}

function renderObject(seg, ctx) {
  const { object } = seg;
  const el = document.createElement("span");
  el.dataset.kind = "object";
  el.dataset.i = String(seg.inline);
  el.dataset.object = object.kind;
  el.contentEditable = "false";
  el.className = `obj obj-${object.kind}`;
  switch (object.kind) {
    case "image": {
      const { image } = object;
      if (image.part && ctx.mediaUrl) {
        const img = document.createElement("img");
        img.src = ctx.mediaUrl(image.part);
        img.alt = object.label;
        if (image.width_emu) img.width = Math.max(1, Math.round(image.width_emu / 9525));
        if (image.height_emu) img.height = Math.max(1, Math.round(image.height_emu / 9525));
        img.draggable = false;
        el.appendChild(img);
      } else {
        el.textContent = `[${object.label}]`;
      }
      el.title = object.label;
      break;
    }
    case "link":
      el.textContent = object.label;
      el.title = object.url || (object.anchor ? `Link to “${object.anchor}”` : "Link");
      break;
    case "hidden":
      el.setAttribute("aria-hidden", "true");
      break;
    default:
      el.textContent = object.label;
      el.title = object.kind === "preserved" ? "Kept as in the original document (not editable yet)" : object.label;
  }
  return el;
}

/** Renders a paragraph's inlines into `el` (replacing its content). */
export function renderInlines(el, paragraph, ctx) {
  const segments = paragraphSegments(paragraph);
  const nodes = [];
  for (const seg of segments) {
    if (seg.kind === "text") {
      const span = document.createElement("span");
      span.dataset.kind = "run";
      span.dataset.i = String(seg.inline);
      applyRunStyle(span, seg.props);
      appendText(span, seg.text);
      nodes.push(span);
    } else {
      nodes.push(renderObject(seg, ctx));
    }
  }
  el.replaceChildren(...nodes);
  // An empty paragraph (or one with only invisible content) needs a
  // placeholder line break so the caret can be placed in it.
  const visible = segments.some((s) => s.kind === "text" ? s.text : s.object.visible);
  if (!visible) {
    const br = document.createElement("br");
    br.dataset.placeholder = "";
    el.appendChild(br);
  }
}

export function renderParagraph(paragraph, index, ctx, editable = true) {
  const el = document.createElement("p");
  el.className = "para";
  if (index !== null) {
    el.dataset.block = String(index);
    el.dataset.kind = "paragraph";
  }
  const cls = styleClass(paragraph.style_id, ctx.styles);
  if (cls) el.classList.add(cls);
  if (paragraph.numbering) {
    el.classList.add("para-list");
    el.dataset.level = String(paragraph.numbering.level);
  }
  if (!editable) el.contentEditable = "false";
  applyParagraphStyle(el, paragraph);
  renderInlines(el, paragraph, ctx);
  return el;
}

function renderNested(blocks, ctx) {
  return blocks.map((block) => {
    if (block.type === "paragraph") return renderParagraph(block, null, ctx, false);
    if (block.type === "table") return renderTable(block, null, ctx);
    return renderPreserved(block, null);
  });
}

function renderTable(table, index, ctx) {
  const wrap = document.createElement("div");
  wrap.className = "table-wrap";
  wrap.contentEditable = "false";
  if (index !== null) {
    wrap.dataset.block = String(index);
    wrap.dataset.kind = "table";
  }
  wrap.title = "Tables can't be edited yet; they are kept as they are.";
  const el = document.createElement("table");
  for (const row of table.rows || []) {
    const tr = document.createElement("tr");
    for (const cell of row.cells || []) {
      const td = document.createElement("td");
      if (cell.span > 1) td.colSpan = cell.span;
      if (cell.vertical_merge === "continue") td.classList.add("merged-continue");
      td.append(...renderNested(cell.blocks || [], ctx));
      tr.appendChild(td);
    }
    el.appendChild(tr);
  }
  wrap.appendChild(el);
  return wrap;
}

const PRESERVED_LABELS = { sdt: "Content control", customXml: "Custom XML", AlternateContent: "Alternate content" };

function renderPreserved(block, index) {
  const el = document.createElement("div");
  el.className = "preserved-block";
  el.contentEditable = "false";
  if (index !== null) {
    el.dataset.block = String(index);
    el.dataset.kind = "other";
  }
  el.title = `${PRESERVED_LABELS[block.kind] || "Preserved content"} — kept as in the original document`;
  el.textContent = block.text || PRESERVED_LABELS[block.kind] || "Preserved content";
  return el;
}

/** Renders all blocks of the model into `root`. */
export function renderBlocks(root, model, ctx) {
  const context = { ...ctx, styles: model.styles };
  const nodes = (model.blocks || []).map((block, index) => {
    if (block.type === "paragraph") return renderParagraph(block, index, context);
    if (block.type === "table") return renderTable(block, index, context);
    return renderPreserved(block, index);
  });
  root.replaceChildren(...nodes);
}
