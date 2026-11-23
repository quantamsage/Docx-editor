// Find and replace over the document model. No DOM access, so it is
// unit-tested in Node.
//
// The searched text is the visible text of each paragraph as the editor
// shows it (content.js segments): text runs, with every object (picture,
// link, field, …) counting as one character that never matches. Matches
// never cross paragraphs. Paragraphs are searched in the body (including
// the cells of its tables, in order), then in the header and footer.

import { OBJECT_CHAR, paragraphSegments, segmentsText, segmentsToPieces } from "./content.js";
import { cellKey, containerField, containerKey } from "./containers.js";

/** Longest search text. */
export const MAX_QUERY = 255;

const WORD = "[\\p{L}\\p{N}_]";

/** Paragraphs to search, in document order: [{key, block, paragraph}]. */
export function searchableParagraphs(model) {
  const out = [];
  const walk = (blocks, key) => {
    (blocks || []).forEach((block, index) => {
      if (block.type === "paragraph") out.push({ key, block: index, paragraph: block });
      else if (block.type === "table") {
        (block.rows || []).forEach((row, r) =>
          (row.cells || []).forEach((cell, c) => {
            for (const [i, inner] of (cell.blocks || []).entries()) {
              if (inner.type === "paragraph") out.push({ key: cellKey(key, index, r, c), block: i, paragraph: inner });
            }
          }),
        );
      }
    });
  };
  walk(model.blocks, "");
  for (const hf of [model.header, model.footer]) {
    if (hf) walk(hf.blocks, containerKey({ part: hf.part }));
  }
  return out;
}

/** A regular expression for the query, or null when it can't match anything. */
export function searchPattern(query, { matchCase = false, wholeWord = false } = {}) {
  if (!query || query.length > MAX_QUERY || query.includes(OBJECT_CHAR) || /[\r\n]/.test(query)) return null;
  const escaped = query.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const source = wholeWord ? `(?<!${WORD})${escaped}(?!${WORD})` : escaped;
  return new RegExp(source, `g${matchCase ? "" : "i"}u`);
}

/**
 * All matches of `query`: [{key, block, start, end}] with character
 * offsets as the editor counts them.
 */
export function findMatches(model, query, options = {}) {
  const pattern = searchPattern(query, options);
  if (!pattern) return [];
  const matches = [];
  for (const { key, block, paragraph } of searchableParagraphs(model)) {
    const text = segmentsText(paragraphSegments(paragraph));
    for (const m of text.matchAll(pattern)) {
      if (m[0].length) matches.push({ key, block, start: m.index, end: m.index + m[0].length });
    }
  }
  return matches;
}

/**
 * Splice content for paragraph `block` with the ranges `ranges` ([{start,
 * end}], in order, not overlapping) replaced by `replacement`. Replacement
 * text takes the formatting of the text where the match starts; text
 * outside the matches keeps its runs; objects are kept.
 */
export function replaceInParagraph(block, paragraph, ranges, replacement) {
  const segments = paragraphSegments(paragraph);
  const out = [];
  const push = (seg) => {
    const prev = out[out.length - 1];
    if (seg.kind === "text" && prev && prev.kind === "text" && prev.inline === seg.inline) prev.text += seg.text;
    else if (seg.kind === "object" || seg.text) out.push({ ...seg });
  };
  let pos = 0;
  let r = 0;
  for (const seg of segments) {
    const len = seg.kind === "text" ? seg.text.length : 1;
    if (seg.kind === "object") {
      push({ kind: "object", inline: seg.inline });
      pos += len;
      continue;
    }
    for (let i = 0; i < len; i++) {
      const at = pos + i;
      while (r < ranges.length && ranges[r].end <= at) r++;
      const range = ranges[r];
      if (range && at >= range.start && at < range.end) {
        if (at === range.start) push({ kind: "text", text: replacement, inline: seg.inline });
        continue;
      }
      push({ kind: "text", text: seg.text[i], inline: seg.inline });
    }
    pos += len;
  }
  return segmentsToPieces(block, paragraph, out);
}

/**
 * The edits replacing `matches` (from findMatches on the same model) with
 * `replacement`: one splice per paragraph, all in one batch.
 */
export function replaceEdits(model, matches, replacement) {
  const byParagraph = new Map();
  for (const m of matches) {
    const id = `${m.key}#${m.block}`;
    if (!byParagraph.has(id)) byParagraph.set(id, { key: m.key, block: m.block, ranges: [] });
    byParagraph.get(id).ranges.push({ start: m.start, end: m.end });
  }
  const paragraphs = new Map(searchableParagraphs(model).map((p) => [`${p.key}#${p.block}`, p.paragraph]));
  const edits = [];
  for (const [id, { key, block, ranges }] of byParagraph) {
    const paragraph = paragraphs.get(id);
    if (!paragraph) continue;
    ranges.sort((a, b) => a.start - b.start);
    const content = replaceInParagraph(block, paragraph, ranges, replacement);
    edits.push({ op: "splice", ...containerField(key), first: block, last: block, content });
  }
  return edits;
}

/** "3 of 12", "No results". */
export function matchLabel(index, count) {
  if (!count) return "No results";
  return index >= 0 ? `${index + 1} of ${count}` : `${count} result${count === 1 ? "" : "s"}`;
}
