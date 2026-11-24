// Status-bar logic, kept free of DOM access so it can be unit tested.

/**
 * Counts words the way word processors do: whitespace-separated tokens that
 * contain at least one letter or digit ("3.50", "well-known" and "it's" are
 * one word each; a lone dash is not a word).
 * @param {string} text
 */
export function countWords(text) {
  return String(text ?? "")
    .split(/\s+/u)
    .filter((token) => /[\p{L}\p{N}]/u.test(token)).length;
}

/** "1 word", "12 words". */
export function formatWordCount(n) {
  return `${n.toLocaleString("en-US")} ${n === 1 ? "word" : "words"}`;
}

/** "Page 2 of 5". */
export function formatPagePosition(current, total) {
  const safeTotal = Math.max(1, total | 0);
  const safeCurrent = Math.min(Math.max(1, current | 0), safeTotal);
  return `Page ${safeCurrent} of ${safeTotal}`;
}

/**
 * Text for the connection indicator.
 * @param {{state: "connecting"|"connected"|"offline", version?: string, message?: string}} status
 */
export function connectionLabel(status) {
  switch (status.state) {
    case "connected":
      return status.version ? `Local server v${status.version}` : "Local server connected";
    case "offline":
      return status.message || "Local server unavailable";
    default:
      return "Connecting to local server…";
  }
}

/**
 * Periodically checks the server and reports state changes.
 *
 * @param {object} options
 * @param {() => Promise<{version: string}>} options.check  Health check.
 * @param {(status: object) => void} options.onChange  Called when the state changes.
 * @param {number} [options.intervalMs]  Time between checks while connected.
 * @param {number} [options.retryMs]  Time between checks while offline.
 * @param {(fn: () => void, ms: number) => any} [options.setTimer]
 * @param {(id: any) => void} [options.clearTimer]
 * @returns {{checkNow: () => Promise<void>, stop: () => void}}
 */
export function createConnectionMonitor({
  check,
  onChange,
  intervalMs = 15000,
  retryMs = 3000,
  setTimer = (fn, ms) => setTimeout(fn, ms),
  clearTimer = (id) => clearTimeout(id),
}) {
  let last = { state: "connecting" };
  let timer = null;
  let stopped = false;

  function report(next) {
    if (next.state !== last.state || next.version !== last.version || next.message !== last.message) {
      last = next;
      onChange(next);
    }
  }

  async function checkNow() {
    if (timer !== null) {
      clearTimer(timer);
      timer = null;
    }
    let next;
    try {
      const health = await check();
      next = { state: "connected", version: health.version };
    } catch (err) {
      next = { state: "offline", message: (err && err.message) || "Local server unavailable" };
    }
    if (stopped) return;
    report(next);
    timer = setTimer(checkNow, next.state === "connected" ? intervalMs : retryMs);
  }

  function stop() {
    stopped = true;
    if (timer !== null) clearTimer(timer);
    timer = null;
  }

  return { checkNow, stop };
}

/**
 * Estimates the page count, and the page of the caret, on the continuous
 * sheet the editor shows: the content runs down pages of `pageHeight`, and
 * a page break moves what follows it to the next page, below its top
 * margin.
 *
 * @param {number} height  Height of the whole sheet.
 * @param {number} pageHeight  Height of one page.
 * @param {number[]} breaks  Where page breaks end, from the top of the sheet, in order.
 * @param {number|null} caret  Where the caret is, from the top of the sheet.
 * @param {number} [marginTop]  The top margin of a page.
 * @returns {{current: number, total: number}}
 */
export function estimatePages(height, pageHeight, breaks, caret, marginTop = 0) {
  const size = Math.max(1, pageHeight);
  let shift = 0;
  let caretPage = null;
  for (const at of breaks) {
    if (caret !== null && caretPage === null && caret < at) caretPage = Math.floor((caret + shift) / size) + 1;
    const y = at + shift;
    shift += (Math.floor(y / size) + 1) * size + marginTop - y;
  }
  const total = Math.max(1, Math.ceil((height + shift - 1) / size));
  if (caret !== null && caretPage === null) caretPage = Math.floor((caret + shift) / size) + 1;
  return { total, current: Math.min(total, Math.max(1, caretPage ?? 1)) };
}
