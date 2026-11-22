// The formatting toolbar: shows the formatting at the selection and turns
// button presses, menu choices and keyboard shortcuts into edits.

/** Fonts offered in the font menu (plus the selection's own font). */
export const FONTS = ["Calibri", "Calibri Light", "Arial", "Cambria", "Georgia", "Helvetica", "Segoe UI", "Times New Roman", "Verdana", "Courier New"];

/** Font sizes in points. */
export const SIZES = [8, 9, 10, 10.5, 11, 12, 14, 16, 18, 20, 24, 28, 36, 48, 72];

/** Built-in paragraph styles offered even if the document lacks them (the engine adds them). */
const BUILTIN_STYLES = [
  { id: "Title", name: "Title" },
  { id: "Heading1", name: "Heading 1" },
  { id: "Heading2", name: "Heading 2" },
  { id: "Heading3", name: "Heading 3" },
];

/** A style's display name: "heading 1" → "Heading 1". */
export function styleLabel(style) {
  const name = (style.name || style.id).trim();
  return name.charAt(0).toUpperCase() + name.slice(1);
}

/** Paragraph styles for the style menu: Normal first, then built-ins, then the document's own. */
export function styleOptions(styles) {
  const paragraph = (styles || []).filter((s) => s.kind === "paragraph" && s.id !== "Normal");
  const options = [{ id: "", label: "Normal" }];
  const seen = new Set();
  for (const builtin of BUILTIN_STYLES) {
    const own = paragraph.find((s) => s.id === builtin.id);
    options.push({ id: builtin.id, label: own ? styleLabel(own) : builtin.name });
    seen.add(builtin.id);
  }
  for (const s of paragraph.sort((a, b) => styleLabel(a).localeCompare(styleLabel(b)))) {
    if (!seen.has(s.id)) options.push({ id: s.id, label: styleLabel(s) });
  }
  return options;
}

/** The new indent (twips) after an increase (+1) or decrease (-1) by half an inch. */
export function steppedIndent(current, step) {
  // Move to the next half-inch stop in that direction, as Word does.
  const value = current || 0;
  const next = step > 0 ? (Math.floor(value / 720) + 1) * 720 : Math.ceil(value / 720 - 1) * 720;
  return Math.max(0, Math.min(next, 31680));
}

/** The next font size (half-points) up or down the size list. */
export function steppedSize(currentHalfPoints, step) {
  const current = (currentHalfPoints || 22) / 2;
  const list = step > 0 ? SIZES.filter((s) => s > current) : SIZES.filter((s) => s < current).reverse();
  const next = list[0] ?? current;
  return Math.round(next * 2);
}

/** Keyboard shortcut → toolbar command, or null. */
export function shortcutFor(event) {
  const mod = event.ctrlKey || event.metaKey;
  if (!mod) return null;
  const key = event.key.toLowerCase();
  if (event.altKey && !event.shiftKey) {
    return { 1: "heading1", 2: "heading2", 3: "heading3", 0: "normal" }[key] || null;
  }
  if (event.altKey) return null;
  if (event.shiftKey) {
    // Shift changes the key of digits ("8" → "*"), so digits go by code.
    const byKey = { l: "align-left", e: "align-center", r: "align-right", j: "align-justify", x: "strike", m: "outdent", n: "normal" };
    const byCode = { Digit8: "bullets", Digit7: "numbering" };
    return byKey[key] || byCode[event.code] || null;
  }
  return { b: "bold", i: "italic", u: "underline", "]": "grow", "[": "shrink", m: "indent" }[key] || null;
}

function option(value, label) {
  const o = document.createElement("option");
  o.value = value;
  o.textContent = label;
  return o;
}

/**
 * @param {HTMLElement} bar  The toolbar element (controls carry data-format).
 * @param {ReturnType<import("./editor.js").createEditor>} editor
 */
export function createToolbar(bar, editor) {
  const $ = (selector) => bar.querySelector(selector);
  const controls = () => bar.querySelectorAll("[data-format]");
  let current = null;
  let lastStyles = null;

  function fillStyles(styles) {
    if (styles === lastStyles) return;
    lastStyles = styles;
    $("#style-select").replaceChildren(...styleOptions(styles).map((s) => option(s.id, s.label)));
  }

  function fillFonts(font) {
    const select = $("#font-family");
    const fonts = font && !FONTS.includes(font) ? [font, ...FONTS] : FONTS;
    select.replaceChildren(option("", "(Style font)"), ...fonts.map((f) => option(f, f)));
    select.value = font || "";
  }

  function fillSizes(size) {
    const select = $("#font-size");
    const points = size ? size / 2 : null;
    const sizes = points && !SIZES.includes(points) ? [...SIZES, points].sort((a, b) => a - b) : SIZES;
    select.replaceChildren(option("", "–"), ...sizes.map((s) => option(String(Math.round(s * 2)), String(s))));
    select.value = size ? String(size) : "";
  }

  /** Reflects the formatting at the selection (or disables everything). */
  function update() {
    current = editor.currentFormat();
    for (const control of controls()) control.disabled = !current;
    if (!current) return;
    const { run, paragraph, list } = current;
    fillStyles(editor.document.model.styles);
    $("#style-select").value = paragraph.style_id && [...$("#style-select").options].some((o) => o.value === paragraph.style_id) ? paragraph.style_id : "";
    fillFonts(run.font);
    fillSizes(run.size);
    for (const name of ["bold", "italic", "underline", "strike"]) {
      $(`[data-format="${name}"]`).setAttribute("aria-pressed", String(Boolean(run[name])));
    }
    $("#text-color").value = run.color ? `#${run.color.toLowerCase()}` : "#000000";
    $("#highlight-select").value = run.highlight && [...$("#highlight-select").options].some((o) => o.value === run.highlight) ? run.highlight : "";
    const alignment = paragraph.alignment || "left";
    for (const button of bar.querySelectorAll('[data-format="align"]')) button.setAttribute("aria-pressed", String(button.dataset.value === alignment));
    for (const button of bar.querySelectorAll('[data-format="list"]')) button.setAttribute("aria-pressed", String(button.dataset.value === list));
  }

  const after = (promise) => promise.then(update, update);

  /** Runs a toolbar command by name (also used for keyboard shortcuts). */
  function command(name, value) {
    if (!editor.document) return;
    const state = current || editor.currentFormat();
    if (!state) return;
    const run = state.run || {};
    switch (name) {
      case "bold":
      case "italic":
      case "underline":
      case "strike":
        return after(editor.formatText({ [name]: !run[name] }));
      case "font":
        return after(editor.formatText({ font: value || null }));
      case "size":
        return value ? after(editor.formatText({ size: Number(value) })) : undefined;
      case "grow":
      case "shrink":
        return after(editor.formatText({ size: steppedSize(run.size, name === "grow" ? 1 : -1) }));
      case "color":
        return after(editor.formatText({ color: value.replace("#", "").toUpperCase() }));
      case "highlight":
        return after(editor.formatText({ highlight: value || null }));
      case "style":
        return after(editor.formatParagraphs({ style_id: value || null }));
      case "heading1":
      case "heading2":
      case "heading3":
        return after(editor.formatParagraphs({ style_id: `Heading${name.slice(-1)}` }));
      case "normal":
        return after(editor.formatParagraphs({ style_id: null }));
      case "align":
        return after(editor.formatParagraphs({ alignment: value }));
      case "align-left":
      case "align-center":
      case "align-right":
      case "align-justify":
        return command("align", name.slice(6));
      case "bullets":
      case "numbering":
        return command("list", name === "bullets" ? "bullet" : "number");
      case "list": {
        const off = state.list === value;
        return after(editor.formatParagraphs((p) => ({ list: off ? null : { kind: value, level: p.numbering ? p.numbering.level : 0 } })));
      }
      case "indent":
      case "outdent": {
        const step = name === "indent" ? 1 : -1;
        return after(
          editor.formatParagraphs((p) =>
            p.numbering
              ? { list: { kind: listKindOf(p), level: Math.max(0, Math.min(8, (p.numbering.level || 0) + step)) } }
              : { indent: { left: steppedIndent(p.indent && p.indent.left, step) } },
          ),
        );
      }
      case "line":
        return value ? after(editor.formatParagraphs({ spacing: { line: Number(value) } })) : undefined;
      case "paragraph-spacing": {
        if (!value) return undefined;
        const [before, afterSpace] = value.split(",").map(Number);
        return after(editor.formatParagraphs({ spacing: { before, after: afterSpace } }));
      }
      default:
        return undefined;
    }
  }

  function listKindOf(p) {
    const def = (editor.document.model.lists || []).find((d) => d.num_id === p.numbering.num_id);
    return def && def.levels[0] && def.levels[0].format === "bullet" ? "bullet" : "number";
  }

  // Buttons keep the editor's selection: act on mousedown, not focus.
  bar.addEventListener("mousedown", (event) => {
    if (event.target.closest("button[data-format]")) event.preventDefault();
  });
  bar.addEventListener("click", (event) => {
    const button = event.target.closest("button[data-format]");
    if (!button || button.disabled) return;
    const name = button.dataset.format;
    if (name === "indent") command(button.dataset.value === "1" ? "indent" : "outdent");
    else command(name, button.dataset.value);
  });
  bar.addEventListener("change", (event) => {
    const control = event.target.closest("[data-format]");
    if (!control || control.tagName === "BUTTON") return;
    const value = control.value;
    command(control.dataset.format, value);
    if (control.dataset.format === "line" || control.dataset.format === "paragraph-spacing") control.value = "";
  });

  return {
    update,
    command,
    /** Handles a keydown in the document; returns true if it was a shortcut. */
    handleKey(event) {
      const name = shortcutFor(event);
      if (!name || !editor.document) return false;
      event.preventDefault();
      command(name);
      return true;
    },
  };
}
