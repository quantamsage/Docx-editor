import { test } from "node:test";
import assert from "node:assert/strict";

import { MAX_DRAFT_AGE_MS, claimDraft, createDraftStore, draftMessage, draftsToOffer, heldLocks, lockName, newDraftId } from "../../web/js/drafts.js";
import { saveErrorMessage } from "../../web/js/files.js";

const now = Date.UTC(2026, 10, 20, 14, 5);

test("drafts open in another tab or too old are not offered", () => {
  const drafts = [
    { id: "a", name: "A.docx", savedAt: now - 60_000 },
    { id: "b", name: "B.docx", savedAt: now - 1000 },
    { id: "c", name: "C.docx", savedAt: now - MAX_DRAFT_AGE_MS - 1 },
    { id: "d", name: "D.docx", savedAt: now - 5000 },
  ];
  const { offer, expired } = draftsToOffer(drafts, new Set([lockName("d")]), now);
  assert.deepEqual(offer.map((d) => d.id), ["b", "a"], "newest first; d is open elsewhere");
  assert.deepEqual(expired.map((d) => d.id), ["c"]);
  assert.match(draftMessage(drafts[1], now), /^Unsaved changes to “B\.docx” from .+ can be recovered\.$/);
});

test("locks are claimed, listed and released", async () => {
  const held = new Map();
  const locks = {
    request(name, options, callback) {
      held.set(name, options.mode);
      return callback().then(() => held.delete(name));
    },
    query: async () => ({ held: [...held.keys()].map((name) => ({ name })) }),
  };
  const claim = claimDraft("x1", locks);
  assert.deepEqual([...(await heldLocks(locks))], [lockName("x1")]);
  claim.release();
  await new Promise((r) => setTimeout(r, 0));
  assert.equal((await heldLocks(locks)).size, 0);
  // Without Web Locks nothing is held and claiming is harmless.
  assert.equal((await heldLocks(undefined)).size, 0);
  claimDraft("y", undefined).release();
  assert.notEqual(newDraftId(), newDraftId());
});

test("draft storage fails softly without IndexedDB", async () => {
  const store = createDraftStore(undefined);
  assert.deepEqual(await store.list(), []);
  assert.equal(await store.get("a"), undefined);
  await store.put({ id: "a", name: "A.docx", savedAt: now });
  await store.remove("a");
});

test("save errors are explained", () => {
  assert.match(saveErrorMessage({ name: "NotAllowedError" }, "a.docx"), /Permission to write to “a\.docx”/);
  assert.match(saveErrorMessage({ name: "NoModificationAllowedError" }, "a.docx"), /open in another program/);
  assert.match(saveErrorMessage({ name: "QuotaExceededError" }, "a.docx"), /disk is full/);
  assert.match(saveErrorMessage({ name: "TypeError", message: "boom" }, "a.docx"), /couldn't be saved: boom/);
});
