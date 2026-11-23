// Page layout and picture sizes: unit conversions, paper sizes and margin
// presets for the Page setup dialog, and fitting pictures on the page. No
// DOM access, so it is unit-tested in Node.

export const EMU_PER_PIXEL = 9525;
export const EMU_PER_TWIP = 635;
export const TWIPS_PER_CM = 1440 / 2.54;

/** Paper sizes offered in Page setup (portrait twips). */
export const PAPER_SIZES = [
  { id: "a4", label: "A4 (21 × 29.7 cm)", width: 11906, height: 16838 },
  { id: "letter", label: "Letter (8.5 × 11 in)", width: 12240, height: 15840 },
  { id: "legal", label: "Legal (8.5 × 14 in)", width: 12240, height: 20160 },
  { id: "a5", label: "A5 (14.8 × 21 cm)", width: 8391, height: 11906 },
  { id: "a3", label: "A3 (29.7 × 42 cm)", width: 16838, height: 23811 },
];

/** Margin presets (twips), named as in Word. */
export const MARGIN_PRESETS = [
  { id: "normal", label: "Normal (2.54 cm)", top: 1440, right: 1440, bottom: 1440, left: 1440 },
  { id: "narrow", label: "Narrow (1.27 cm)", top: 720, right: 720, bottom: 720, left: 720 },
  { id: "moderate", label: "Moderate", top: 1440, right: 1080, bottom: 1440, left: 1080 },
  { id: "wide", label: "Wide", top: 1440, right: 2880, bottom: 1440, left: 2880 },
];

const TOLERANCE = 60;

/** Twips to centimetres, rounded to 0.01. */
export function twipsToCm(twips) {
  return Math.round((twips / TWIPS_PER_CM) * 100) / 100;
}

/** Centimetres to whole twips. */
export function cmToTwips(cm) {
  return Math.round(Number(cm) * TWIPS_PER_CM);
}

/** The paper size id of a section ("custom" if it isn't a listed size). */
export function paperId(section) {
  const w = (section && section.page_width) || 11906;
  const h = (section && section.page_height) || 16838;
  const short = Math.min(w, h);
  const long = Math.max(w, h);
  const found = PAPER_SIZES.find((p) => Math.abs(p.width - short) <= TOLERANCE && Math.abs(p.height - long) <= TOLERANCE);
  return found ? found.id : "custom";
}

/** The margin preset id matching a section's margins, or "custom". */
export function marginPresetId(section) {
  const m = marginsOf(section);
  const found = MARGIN_PRESETS.find((p) => ["top", "right", "bottom", "left"].every((side) => Math.abs(p[side] - m[side]) <= 10));
  return found ? found.id : "custom";
}

/** A section's margins in twips (Word's defaults where missing). */
export function marginsOf(section) {
  const s = section || {};
  const pick = (v, d) => (v === undefined || v === null ? d : Math.abs(v));
  return { top: pick(s.margin_top, 1440), right: pick(s.margin_right, 1440), bottom: pick(s.margin_bottom, 1440), left: pick(s.margin_left, 1440) };
}

/**
 * The format_section format for Page setup choices. `margins` are in
 * centimetres. Returns {format} or {error}.
 */
export function sectionFormat({ paper, landscape, margins, section }) {
  const size = PAPER_SIZES.find((p) => p.id === paper);
  const twips = {};
  for (const side of ["top", "right", "bottom", "left"]) {
    const cm = Number(margins[side]);
    if (!Number.isFinite(cm) || cm < 0 || cm > 55) return { error: "Margins must be between 0 and 55 cm." };
    twips[side] = cmToTwips(cm);
  }
  // Portrait dimensions of the chosen paper (or of the current, custom one).
  const current = section || {};
  const curW = current.page_width || 11906;
  const curH = current.page_height || 16838;
  const short = size ? size.width : Math.min(curW, curH);
  const long = size ? size.height : Math.max(curW, curH);
  const pageW = landscape ? long : short;
  const pageH = landscape ? short : long;
  if (twips.left + twips.right + 360 > pageW || twips.top + twips.bottom + 360 > pageH) {
    return { error: "The margins leave no room for text on the page." };
  }
  const format = { landscape: Boolean(landscape), margins: twips };
  if (size) format.size = { width: size.width, height: size.height };
  return { format };
}

/** Width available for text, in twips. */
export function textWidthTwips(section) {
  const width = (section && section.page_width) || 11906;
  const m = marginsOf(section);
  return Math.max(720, width - m.left - m.right);
}

/**
 * Display size of a new picture: its pixel size at 96 dpi, scaled down to
 * fit the text width if needed.
 * @returns {{width_emu: number, height_emu: number}}
 */
export function pictureSize(widthPx, heightPx, section) {
  const maxEmu = textWidthTwips(section) * EMU_PER_TWIP;
  let w = Math.max(1, widthPx) * EMU_PER_PIXEL;
  let h = Math.max(1, heightPx) * EMU_PER_PIXEL;
  if (w > maxEmu) {
    h = Math.round((h * maxEmu) / w);
    w = maxEmu;
  }
  return { width_emu: Math.max(1, Math.round(w)), height_emu: Math.max(1, Math.round(h)) };
}

/**
 * A picture's new size while its corner handle is dragged by `dx` pixels,
 * keeping its proportions (and at least 8 px).
 * @returns {{width: number, height: number}}  In CSS pixels.
 */
export function resizedPicture(startWidth, startHeight, dx, maxWidth = Infinity) {
  const ratio = startHeight / Math.max(1, startWidth);
  const width = Math.round(Math.min(maxWidth, Math.max(8, startWidth + dx)));
  return { width, height: Math.max(1, Math.round(width * ratio)) };
}
