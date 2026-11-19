// Interpretation of the document model sent by the server. No DOM access.

const TWIPS_PER_INCH = 1440;
const CSS_PX_PER_INCH = 96;
const TWIPS_PER_MM = TWIPS_PER_INCH / 25.4;
/** Tolerance when recognising standard paper sizes (about 1 mm). */
const SIZE_TOLERANCE = 60;

const PAPER_SIZES = [
  { name: "A4", short: 11906, long: 16838 },
  { name: "Letter", short: 12240, long: 15840 },
  { name: "Legal", short: 12240, long: 20160 },
  { name: "A5", short: 8391, long: 11906 },
  { name: "A3", short: 16838, long: 23811 },
  { name: "Executive", short: 10440, long: 15120 },
];

const DEFAULT_SECTION = {
  page_width: 11906,
  page_height: 16838,
  margin_top: 1440,
  margin_right: 1440,
  margin_bottom: 1440,
  margin_left: 1440,
};

/** Converts twentieths of a point to CSS pixels. */
export function twipsToPx(twips) {
  return (twips * CSS_PX_PER_INCH) / TWIPS_PER_INCH;
}

/**
 * Page size, margins (CSS px) and a human-readable label for a section.
 * Missing values fall back to A4 with 1-inch margins, as Word does.
 */
export function pageGeometry(section) {
  const s = { ...DEFAULT_SECTION, ...stripNulls(section || {}) };
  let width = s.page_width;
  let height = s.page_height;
  const landscape = section && section.landscape ? true : width > height;
  if (landscape && width < height) [width, height] = [height, width];

  const short = Math.min(width, height);
  const long = Math.max(width, height);
  const paper = PAPER_SIZES.find((p) => Math.abs(p.short - short) <= SIZE_TOLERANCE && Math.abs(p.long - long) <= SIZE_TOLERANCE);
  const sizeName = paper ? paper.name : `${Math.round(width / TWIPS_PER_MM)} × ${Math.round(height / TWIPS_PER_MM)} mm`;

  return {
    widthPx: twipsToPx(width),
    heightPx: twipsToPx(height),
    margins: {
      top: twipsToPx(Math.abs(s.margin_top)),
      right: twipsToPx(s.margin_right),
      bottom: twipsToPx(Math.abs(s.margin_bottom)),
      left: twipsToPx(s.margin_left),
    },
    sizeName,
    landscape,
    label: `${sizeName}, ${landscape ? "landscape" : "portrait"}`,
  };
}

function stripNulls(obj) {
  return Object.fromEntries(Object.entries(obj).filter(([, v]) => v !== null && v !== undefined));
}

/** Visible text of a paragraph from the model. */
export function paragraphText(paragraph) {
  let out = "";
  for (const inline of paragraph.inlines || []) {
    if (inline.type === "other") {
      out += inline.text || "";
      continue;
    }
    const runs = inline.type === "hyperlink" ? inline.runs : [inline];
    for (const run of runs) {
      for (const item of run.content || []) {
        if (item.type === "text") out += item.text;
        else if (item.type === "tab") out += "\t";
        else if (item.type === "line_break") out += "\n";
      }
    }
  }
  return out;
}

function plural(n, word, pluralWord = `${word}s`) {
  return `${n.toLocaleString("en-US")} ${n === 1 ? word : pluralWord}`;
}

/**
 * Short descriptions of what a document contains, e.g.
 * ["12 paragraphs", "1 table", "2 images"]. Zero counts are omitted.
 */
export function summarize(model) {
  const stats = model.stats || {};
  const items = [
    [stats.paragraphs, "paragraph"],
    [stats.tables, "table"],
    [stats.images, "image"],
    [stats.hyperlinks, "hyperlink"],
    [(model.headers || []).length, "header"],
    [(model.footers || []).length, "footer"],
  ];
  return items.filter(([n]) => n > 0).map(([n, word]) => plural(n, word));
}

/** Sentence explaining preserved content, or "" if there is none. */
export function preservedNote(model) {
  const n = (model.stats && model.stats.preserved_elements) || 0;
  if (n === 0) return "";
  return `${plural(n, "element")} the editor does not display yet (such as fields, comments or content controls) will be kept unchanged when you save.`;
}

/**
 * Headings for an outline: paragraphs styled Title or Heading 1–9, matched
 * by style id or by style name (Word localizes neither for built-in ids).
 * @returns {{level: number, text: string}[]}
 */
export function outline(model, limit = 50) {
  const styleNames = new Map((model.styles || []).map((s) => [s.id, s.name]));
  const result = [];
  for (const block of model.blocks || []) {
    if (block.type !== "paragraph" || !block.style_id) continue;
    const level = headingLevel(block.style_id) ?? headingLevel(styleNames.get(block.style_id) || "");
    const text = paragraphText(block).trim();
    if (level !== null && text) {
      result.push({ level, text });
      if (result.length >= limit) break;
    }
  }
  return result;
}

function headingLevel(name) {
  const compact = name.replace(/\s+/g, "").toLowerCase();
  if (compact === "title") return 0;
  const match = /^heading([1-9])$/.exec(compact);
  return match ? Number(match[1]) : null;
}

/** Ensures a file name ends with a Word extension. */
export function ensureDocxExtension(name) {
  const trimmed = String(name || "").trim();
  if (!trimmed) return "Untitled document.docx";
  return /\.(docx|docm|dotx|dotm)$/i.test(trimmed) ? trimmed : `${trimmed}.docx`;
}
