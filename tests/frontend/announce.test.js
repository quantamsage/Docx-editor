import { test } from "node:test";
import assert from "node:assert/strict";

import { createAnnouncer, openedMessage } from "../../web/js/announce.js";

test("messages go to the polite or the urgent region, and repeats are read again", (t) => {
  t.mock.timers.enable({ apis: ["setTimeout"] });
  const polite = { textContent: "" };
  const urgent = { textContent: "" };
  const announce = createAnnouncer(polite, urgent);
  announce("Saved report.docx.");
  assert.equal(polite.textContent, "", "the region is cleared first");
  t.mock.timers.tick(60);
  assert.equal(polite.textContent, "Saved report.docx.");
  announce("The file could not be saved.", { urgent: true });
  t.mock.timers.tick(60);
  assert.equal(urgent.textContent, "The file could not be saved.");
  assert.equal(polite.textContent, "", "only one region speaks at a time");
  announce("Undone.");
  t.mock.timers.tick(60);
  announce("Undone.");
  assert.equal(polite.textContent, "", "the same message is cleared before it is repeated");
  t.mock.timers.tick(60);
  assert.equal(polite.textContent, "Undone.");
});

test("opening and creating documents are described", () => {
  assert.equal(openedMessage("report.docx", 1234, false), "Opened report.docx, 1,234 words.");
  assert.equal(openedMessage("note.docx", 1, false), "Opened note.docx, 1 word.");
  assert.equal(openedMessage("Untitled document.docx", 0, true), "New blank document, Untitled document.docx.");
});
