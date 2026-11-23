// Local draft recovery.
//
// While a document has unsaved changes, a copy of it (the .docx the server
// would save) is kept in the browser's IndexedDB for this site, a few
// seconds after each change. If the browser or the editor stops before the
// document is saved, the next start offers to recover the draft. Drafts are
// removed when the document is saved or its changes are discarded. They
// never leave the computer.
//
// Each open document owns its draft through a Web Lock (when the browser
// has them), so a draft that another tab is still editing is not offered.

export const DB_NAME = "docx-editor-drafts";
const STORE = "drafts";
/** Drafts older than this are deleted instead of offered. */
export const MAX_DRAFT_AGE_MS = 30 * 24 * 60 * 60 * 1000;
/** How long after a change the draft is written. */
export const DRAFT_DELAY_MS = 2000;

/** The lock name owning a draft. */
export const lockName = (id) => `docx-editor-draft-${id}`;

/**
 * Which drafts to offer: those not owned by an open tab and not too old,
 * newest first. Returns {offer, expired}.
 * @param {Array<{id: string, savedAt: number}>} drafts
 * @param {Set<string>} heldLocks  Names of locks held by open tabs.
 */
export function draftsToOffer(drafts, heldLocks, now = Date.now()) {
  const expired = drafts.filter((d) => now - d.savedAt > MAX_DRAFT_AGE_MS);
  const offer = drafts
    .filter((d) => now - d.savedAt <= MAX_DRAFT_AGE_MS && !heldLocks.has(lockName(d.id)))
    .sort((a, b) => b.savedAt - a.savedAt);
  return { offer, expired };
}

/** "Unsaved changes to “Report.docx” from 14:05 can be recovered." */
export function draftMessage(draft, now = Date.now()) {
  const date = new Date(draft.savedAt);
  const sameDay = new Date(now).toDateString() === date.toDateString();
  const time = date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const when = sameDay ? time : `${date.toLocaleDateString()} ${time}`;
  return `Unsaved changes to “${draft.name}” from ${when} can be recovered.`;
}

function promised(request) {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

/**
 * Draft storage in IndexedDB. Every method fails softly (resolving with an
 * empty result) when storage is unavailable, e.g. in a private window.
 * @param {IDBFactory|undefined} idb
 */
export function createDraftStore(idb = globalThis.indexedDB) {
  let db = null;
  function open() {
    if (!idb) return Promise.resolve(null);
    if (!db) {
      const request = idb.open(DB_NAME, 1);
      request.onupgradeneeded = () => request.result.createObjectStore(STORE, { keyPath: "id" });
      db = promised(request).catch(() => null);
    }
    return db;
  }
  async function run(mode, action) {
    const database = await open();
    if (!database) return undefined;
    const tx = database.transaction(STORE, mode);
    const result = await promised(action(tx.objectStore(STORE)));
    return result;
  }
  return {
    /** Stores or replaces a draft {id, name, savedAt, data: Blob}. */
    put: (draft) => run("readwrite", (s) => s.put(draft)).catch(() => undefined),
    get: (id) => run("readonly", (s) => s.get(id)).catch(() => undefined),
    /** All drafts (without their data's contents being read). */
    list: async () => (await run("readonly", (s) => s.getAll()).catch(() => undefined)) || [],
    remove: (id) => run("readwrite", (s) => s.delete(id)).catch(() => undefined),
  };
}

/** Names of the Web Locks held by open tabs (empty when unsupported). */
export async function heldLocks(locks = globalThis.navigator?.locks) {
  if (!locks || typeof locks.query !== "function") return new Set();
  try {
    const state = await locks.query();
    return new Set((state.held || []).map((l) => l.name));
  } catch {
    return new Set();
  }
}

/**
 * Holds the lock of a draft until `release()` is called (or the page
 * closes). Does nothing when Web Locks are unsupported.
 */
export function claimDraft(id, locks = globalThis.navigator?.locks) {
  let release = () => {};
  if (locks && typeof locks.request === "function") {
    const held = new Promise((resolve) => {
      release = resolve;
    });
    locks.request(lockName(id), { mode: "exclusive" }, () => held).catch(() => {});
  }
  return { release: () => release() };
}

/** A new draft id. */
export function newDraftId() {
  if (globalThis.crypto && typeof globalThis.crypto.randomUUID === "function") return globalThis.crypto.randomUUID();
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2)}`;
}
