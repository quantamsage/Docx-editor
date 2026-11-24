// The find and replace bar (Ctrl+F, Ctrl+H). Matches are found in the
// model (find.js) and highlighted on the page with the CSS Custom
// Highlight API where the browser has it; the current match is also
// scrolled into view. Replacing goes through the editor as ordinary,
// undoable edits.

import { findMatches, matchLabel } from "./find.js";

/**
 * @param {object} options
 * @param {ReturnType<import("./editor.js").createEditor>} options.editor
 * @param {HTMLElement} options.bar
 * @param {(message: string) => void} options.onInfo
 */
export function createFindBar({ editor, bar, onInfo }) {
  const $ = (id) => document.getElementById(id);
  const input = $("find-input");
  const replacement = $("replace-input");
  const highlights = typeof CSS !== "undefined" && CSS.highlights && typeof Highlight !== "undefined" ? CSS.highlights : null;
  let matches = [];
  let index = -1;
  let timer = null;

  const options = () => ({ matchCase: $("find-case").checked, wholeWord: $("find-word").checked });

  function paint() {
    $("find-count").textContent = input.value ? matchLabel(index, matches.length) : "";
    if (!highlights) return;
    highlights.delete("find-matches");
    highlights.delete("find-current");
    if (bar.hidden || !matches.length) return;
    const ranges = matches.map((m) => editor.matchRange(m)).filter(Boolean);
    highlights.set("find-matches", new Highlight(...ranges));
    const current = index >= 0 ? editor.matchRange(matches[index]) : null;
    if (current) highlights.set("find-current", new Highlight(current));
  }

  function reveal() {
    const range = index >= 0 ? editor.matchRange(matches[index]) : null;
    const target = range && range.startContainer.nodeType === 1 ? range.startContainer : range && range.startContainer.parentElement;
    if (target) target.scrollIntoView({ block: "center" });
  }

  /** Finds the matches again (after typing, or a change to the document). */
  async function search({ keepPosition = true } = {}) {
    if (!editor.document) return;
    await editor.flush().catch(() => {});
    const previous = index;
    matches = findMatches(editor.document.model, input.value, options());
    // Stay at the same place in the list: after a replacement, that is the
    // next match.
    index = matches.length ? (keepPosition && previous >= 0 ? Math.min(previous, matches.length - 1) : 0) : -1;
    paint();
    reveal();
  }

  function move(step) {
    if (!matches.length) return;
    index = (index + step + matches.length) % matches.length;
    paint();
    reveal();
  }

  /** The toolbar button that opens the bar (aria-expanded follows the bar). */
  const opener = () => document.querySelector('[aria-controls="find-bar"]');

  function open(withReplace) {
    if (!editor.document) return;
    bar.hidden = false;
    opener()?.setAttribute("aria-expanded", "true");
    $("replace-row").hidden = !withReplace;
    const selection = document.getSelection();
    const selected = selection && !selection.isCollapsed ? selection.toString() : "";
    if (selected && !selected.includes("\n") && selected.length <= 255) input.value = selected;
    (withReplace && input.value ? replacement : input).focus();
    input.select();
    search({ keepPosition: false });
  }

  function close() {
    if (bar.hidden) return;
    bar.hidden = true;
    opener()?.setAttribute("aria-expanded", "false");
    const current = index >= 0 ? matches[index] : null;
    matches = [];
    index = -1;
    paint();
    if (current) editor.selectMatch(current);
    else editor.hosts[0].focus();
  }

  async function replaceOne() {
    if (index < 0) return search();
    const match = matches[index];
    const count = await editor.replaceFound(input.value, options(), replacement.value, match);
    if (!count) onInfo("That text has changed; search again.");
    replacement.focus();
    await search();
  }

  async function replaceAll() {
    if (!input.value) return;
    const count = await editor.replaceFound(input.value, options(), replacement.value);
    onInfo(count ? `Replaced ${count} ${count === 1 ? "match" : "matches"}.` : "Nothing was found to replace.");
    replacement.focus();
    await search({ keepPosition: false });
  }

  input.addEventListener("input", () => {
    clearTimeout(timer);
    timer = setTimeout(() => search({ keepPosition: false }), 150);
  });
  for (const id of ["find-case", "find-word"]) $(id).addEventListener("change", () => search({ keepPosition: false }));
  bar.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.preventDefault();
      close();
    } else if (event.key === "Enter" && event.target === input) {
      event.preventDefault();
      move(event.shiftKey ? -1 : 1);
    } else if (event.key === "Enter" && event.target === replacement) {
      event.preventDefault();
      replaceOne();
    }
  });
  bar.addEventListener("click", (event) => {
    const button = event.target.closest("[data-find]");
    if (!button) return;
    const action = { next: () => move(1), prev: () => move(-1), close, replace: replaceOne, "replace-all": replaceAll }[button.dataset.find];
    if (action) action();
  });

  return {
    open,
    close,
    /** Finds again after the document changed (no-op while closed). */
    refresh() {
      if (!bar.hidden) search();
    },
    get isOpen() {
      return !bar.hidden;
    },
    /** The current matches and index, for tests and the status. */
    get state() {
      return { matches, index };
    },
  };
}
