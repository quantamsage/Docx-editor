// Screen-reader announcements through two live regions that are always in
// the page (a region that is shown or created at the moment it changes is
// often not read). Polite messages wait until the reader is idle; urgent
// ones (errors) interrupt it.

/** Delay before the new text is set, so that repeating a message reads it again. */
const REPEAT_DELAY_MS = 50;

/**
 * @param {HTMLElement} polite  A role="status" element.
 * @param {HTMLElement} urgent  A role="alert" element.
 * @returns {(message: string, options?: {urgent?: boolean}) => void}
 */
export function createAnnouncer(polite, urgent) {
  let timer = null;
  return (message, { urgent: isUrgent = false } = {}) => {
    const region = isUrgent ? urgent : polite;
    clearTimeout(timer);
    polite.textContent = "";
    urgent.textContent = "";
    timer = setTimeout(() => {
      region.textContent = message;
    }, REPEAT_DELAY_MS);
  };
}

/** What to announce when a document has been opened or created. */
export function openedMessage(name, words, isNew) {
  if (isNew) return `New blank document, ${name}.`;
  return `Opened ${name}, ${words.toLocaleString("en-US")} ${words === 1 ? "word" : "words"}.`;
}
