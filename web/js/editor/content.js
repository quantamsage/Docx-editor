// Paragraph content: from the server's model to editable segments, and from
// edited segments back to the engine's splice pieces. No DOM access, so it
// is unit-tested in Node.
//
// A paragraph's inlines (runs, hyperlinks, preserved elements) are shown as
// segments:
//   * "text": a run that only holds text, tabs and line breaks. Its text is
//     editable; new text typed in it keeps its formatting.
//   * "object": anything else (images, hyperlinks, fields, page breaks,
//     preserved content). Objects are shown but not edited; the user can
//     delete visible ones. Invisible ones (field codes, comment references,
//     …) are "sticky": they are always kept, so a field can never be left
//     half-deleted.
//
// Positions inside a paragraph are counted in characters, where every
// object counts as one character.

/** Placeholder character for an object when measuring positions. */
export const OBJECT_CHAR = "￼";

/** In text passed to replaceRange: a line break within the paragraph. */
export const LINE_BREAK = " ";

const TEXT_ITEMS = new Set(["text", "tab", "line_break"]);

/** Text of a run's content items; tabs and line breaks as \t and \n. */
export function runText(run) {
  let out = "";
  for (const item of run.content || []) {
    if (item.type === "text") out += item.text;
    else if (item.type === "tab") out += "\t";
    else if (item.type === "line_break") out += "\n";
  }
  return out;
}

/** Whether a run is shown as editable text (text, tabs and breaks only). */
export function isTextRun(inline) {
  return inline.type === "run" && (inline.content || []).length > 0 && inline.content.every((c) => TEXT_ITEMS.has(c.type));
}

/**
 * Describes how an inline that is not a text run is displayed.
 * @returns {{kind: string, label: string, visible: boolean, image?: object, url?: string|null}}
 */
export function describeObject(inline) {
  if (inline.type === "hyperlink") {
    const text = (inline.runs || []).map(runText).join("");
    return { kind: "link", label: text || inline.url || inline.anchor || "link", visible: true, url: inline.url || null, anchor: inline.anchor || null };
  }
  if (inline.type === "other") {
    const text = inline.text || "";
    return { kind: "preserved", label: text, visible: text.trim() !== "", element: inline.kind };
  }
  // A run with something other than plain text.
  const content = inline.content || [];
  const image = content.find((c) => c.type === "image");
  if (image) return { kind: "image", label: image.description || "Picture", visible: true, image };
  if (content.some((c) => c.type === "page_break")) return { kind: "page_break", label: "Page break", visible: true };
  if (content.some((c) => c.type === "column_break")) return { kind: "column_break", label: "Column break", visible: true };
  const text = runText(inline);
  if (text) return { kind: "preserved", label: text, visible: true };
  const other = content.find((c) => c.type === "other");
  return { kind: "hidden", label: other ? other.kind : "", visible: false };
}

/**
 * Display segments of a paragraph from the model.
 * @returns {Array<{kind: "text", inline: number, text: string, props: object} | {kind: "object", inline: number, object: object}>}
 */
export function paragraphSegments(paragraph) {
  return (paragraph.inlines || []).map((inline, index) =>
    isTextRun(inline)
      ? { kind: "text", inline: index, text: runText(inline), props: inline.props || {} }
      : { kind: "object", inline: index, object: describeObject(inline) },
  );
}

/** Length of segments in characters (objects count as one). */
export function segmentsLength(segments) {
  let n = 0;
  for (const s of segments) n += s.kind === "text" ? s.text.length : 1;
  return n;
}

/** Visible text of segments, with objects as OBJECT_CHAR. */
export function segmentsText(segments) {
  return segments.map((s) => (s.kind === "text" ? s.text : OBJECT_CHAR)).join("");
}

/**
 * Normalizes segments parsed from the editing surface: drops empty text,
 * merges neighbouring text with the same source run, and re-inserts sticky
 * (invisible) objects of the paragraph that are missing.
 *
 * Parsed segments look like `{kind: "text", text, inline}` (inline = index
 * of the run whose formatting the text has, or null) and
 * `{kind: "object", inline}`.
 */
export function normalizeSegments(paragraph, parsed) {
  const out = [];
  const seen = new Set();
  for (const seg of parsed) {
    if (seg.kind === "object") {
      if (seen.has(seg.inline) || !paragraph.inlines[seg.inline]) continue; // duplicates (e.g. copy & paste of the DOM)
      seen.add(seg.inline);
      out.push({ kind: "object", inline: seg.inline });
      continue;
    }
    if (!seg.text) continue;
    const inline = seg.inline !== null && seg.inline !== undefined && isTextRun(paragraph.inlines[seg.inline] || {}) ? seg.inline : null;
    const prev = out[out.length - 1];
    if (prev && prev.kind === "text" && prev.inline === inline) prev.text += seg.text;
    else out.push({ kind: "text", text: seg.text, inline });
  }

  // Sticky objects can't be removed by the user: put back any that went
  // missing, before the first remaining inline that came after them.
  (paragraph.inlines || []).forEach((inline, index) => {
    if (isTextRun(inline) || seen.has(index) || describeObject(inline).visible) return;
    const at = out.findIndex((s) => s.inline !== null && s.inline > index);
    const sticky = { kind: "object", inline: index };
    if (at < 0) out.push(sticky);
    else out.splice(at, 0, sticky);
    seen.add(index);
  });
  return out;
}

/** The run whose formatting new text should take when it has no source. */
function fallbackFormat(paragraph, segments, i) {
  for (let j = i - 1; j >= 0; j--) if (segments[j].kind === "text" && segments[j].inline !== null) return segments[j].inline;
  for (let j = i + 1; j < segments.length; j++) if (segments[j].kind === "text" && segments[j].inline !== null) return segments[j].inline;
  const first = (paragraph.inlines || []).findIndex((x) => x.type === "run");
  return first >= 0 ? first : null;
}

/**
 * Splice pieces for normalized segments of paragraph `block`. Text equal
 * to its source run's full text keeps that run untouched; other text is
 * new, formatted like its source run. Kept inlines are always in document
 * order, as the engine requires.
 */
export function segmentsToPieces(block, paragraph, segments) {
  const pieces = [];
  let lastKept = -1;
  segments.forEach((seg, i) => {
    if (seg.kind === "object") {
      if (seg.inline > lastKept) {
        pieces.push({ type: "keep", block, inline: seg.inline });
        lastKept = seg.inline;
      }
      return;
    }
    const source = seg.inline !== null ? seg.inline : fallbackFormat(paragraph, segments, i);
    const run = source !== null ? paragraph.inlines[source] : null;
    const unchanged = seg.inline !== null && run && isTextRun(run) && runText(run) === seg.text && seg.inline > lastKept;
    if (unchanged) {
      pieces.push({ type: "keep", block, inline: seg.inline });
      lastKept = seg.inline;
    } else {
      const piece = { type: "text", text: seg.text };
      if (source !== null && run && run.type === "run") piece.like = { block, inline: source };
      pieces.push(piece);
    }
  });
  return pieces;
}

/** Whether pieces keep every inline of the paragraph in order and add nothing. */
export function isNoop(block, paragraph, pieces) {
  const n = (paragraph.inlines || []).length;
  return pieces.length === n && pieces.every((p, i) => p.type === "keep" && p.block === block && p.inline === i);
}

/**
 * Splits segments at a character offset into the parts before and after
 * it (objects are never split).
 */
export function splitSegments(segments, offset) {
  const before = [];
  const after = [];
  let pos = 0;
  for (const seg of segments) {
    const len = seg.kind === "text" ? seg.text.length : 1;
    if (pos + len <= offset) before.push(seg);
    else if (pos >= offset) after.push(seg);
    else {
      const cut = offset - pos;
      before.push({ ...seg, text: seg.text.slice(0, cut) });
      after.push({ ...seg, text: seg.text.slice(cut) });
    }
    pos += len;
  }
  return { before, after };
}

/**
 * Builds the pieces for replacing everything between a start position in
 * paragraph `first` and an end position in paragraph `last` with `text`
 * (which may contain newlines: each starts a new paragraph). This is the
 * edit behind Enter, joining paragraphs, deleting a selection and pasting.
 *
 * @param {{block: number, paragraph: object, segments: object[], offset: number}} start
 * @param {{block: number, paragraph: object, segments: object[], offset: number}} end
 * @param {string} text
 * @returns {{splice: {first: number, last: number, content: object[]}, caret: {block: number, offset: number}}}
 */
export function replaceRange(start, end, text) {
  const head = splitSegments(start.segments, start.offset).before;
  const tail = splitSegments(end.segments, end.offset).after;

  // Text typed or pasted takes the formatting at the start of the range.
  const lastHeadText = [...head].reverse().find((s) => s.kind === "text" && s.inline !== null);
  const firstTailText = tail.find((s) => s.kind === "text" && s.inline !== null);
  const likeSource = lastHeadText ? { block: start.block, inline: lastHeadText.inline } : firstTailText ? { block: end.block, inline: firstTailText.inline } : null;

  const content = [];
  const lastKept = new Map();
  const add = (block, paragraph, seg, allSegments, i) => {
    const pieces = segmentsToPieces(block, paragraph, [seg]);
    for (const piece of pieces) {
      if (piece.type === "keep") {
        // A partial run's text (cut at the range edge) is never "unchanged".
        if (seg.kind === "text" && seg.text !== runText(paragraph.inlines[seg.inline])) {
          content.push({ type: "text", text: seg.text, like: { block, inline: seg.inline } });
          continue;
        }
        if ((lastKept.get(block) ?? -1) >= piece.inline) continue;
        lastKept.set(block, piece.inline);
      } else if (!piece.like && seg.kind === "text") {
        const source = fallbackFormat(paragraph, allSegments, i);
        if (source !== null && paragraph.inlines[source]?.type === "run") piece.like = { block, inline: source };
      }
      content.push(piece);
    }
  };
  head.forEach((seg, i) => add(start.block, start.paragraph, seg, head, i));

  // Invisible objects inside the replaced range (field codes, comment
  // references, …) are kept at the edit point rather than deleted.
  const sameBlock = start.block === end.block;
  stickyBetween(start.paragraph, start.segments, start.offset, sameBlock ? end.offset : Infinity).forEach((seg) =>
    add(start.block, start.paragraph, seg, [seg], 0),
  );
  if (!sameBlock) {
    stickyBetween(end.paragraph, end.segments, 0, end.offset).forEach((seg) => add(end.block, end.paragraph, seg, [seg], 0));
  }

  // "\n" separates paragraphs; LINE_BREAK is a line break inside one.
  const lines = text.replace(/\r\n?/g, "\n").split("\n").map((l) => l.replaceAll(LINE_BREAK, "\n"));
  lines.forEach((line, i) => {
    if (i > 0) content.push({ type: "break" });
    if (line) {
      const piece = { type: "text", text: line };
      if (likeSource) piece.like = likeSource;
      content.push(piece);
    }
  });
  tail.forEach((seg, i) => add(end.block, end.paragraph, seg, tail, i));

  const lastLine = lines[lines.length - 1];
  const caret =
    lines.length === 1
      ? { block: start.block, offset: segmentsLength(head) + lastLine.length }
      : { block: start.block + lines.length - 1, offset: lastLine.length };
  return { splice: { first: start.block, last: end.block, content }, caret };
}

/** Invisible objects whose position is in [from, to). */
function stickyBetween(paragraph, segments, from, to) {
  const out = [];
  let pos = 0;
  for (const seg of segments) {
    const len = seg.kind === "text" ? seg.text.length : 1;
    if (seg.kind === "object" && pos >= from && pos < to && !describeObject(paragraph.inlines[seg.inline]).visible) out.push(seg);
    pos += len;
  }
  return out;
}

/**
 * After a splice of one paragraph was applied, maps the paragraph's old
 * inline indices to the new ones, so the editing surface (which may have
 * changed meanwhile) can keep pointing at the right runs.
 *
 * Every keep piece and every non-empty text piece becomes exactly one new
 * inline, in order. Text formatted "like" an old run maps that run to the
 * first new run with its formatting.
 * @returns {Map<number, number>}
 */
export function remapAfterSplice(pieces) {
  const map = new Map();
  let next = 0;
  for (const piece of pieces) {
    if (piece.type === "keep") {
      map.set(piece.inline, next++);
    } else if (piece.type === "text" && piece.text) {
      if (piece.like && !map.has(piece.like.inline)) map.set(piece.like.inline, next);
      next++;
    }
  }
  return map;
}
