import { test } from "node:test";
import assert from "node:assert/strict";

import { DOCX_ACCEPT, downloadBlob, pickFile, pickSaveLocation, supportsFileSystemAccess, writeToHandle } from "../../web/js/files.js";

const abortError = () => Object.assign(new Error("The user aborted a request."), { name: "AbortError" });

test("detects the File System Access API", () => {
  assert.equal(supportsFileSystemAccess({}), false);
  assert.equal(supportsFileSystemAccess({ showOpenFilePicker() {} }), false);
  assert.equal(supportsFileSystemAccess({ showOpenFilePicker() {}, showSaveFilePicker() {} }), true);
});

test("pickFile uses the native picker and keeps the handle", async () => {
  const file = new Blob(["docx"]);
  const handle = { getFile: async () => file };
  let options;
  const win = {
    showOpenFilePicker: async (o) => {
      options = o;
      return [handle];
    },
    showSaveFilePicker() {},
  };
  const picked = await pickFile(win, null);
  assert.equal(picked.file, file);
  assert.equal(picked.handle, handle);
  assert.equal(options.multiple, false);
  assert.deepEqual(options.types[0].accept, DOCX_ACCEPT);
});

test("pickFile returns null when the user cancels", async () => {
  const win = { showOpenFilePicker: async () => Promise.reject(abortError()), showSaveFilePicker() {} };
  assert.equal(await pickFile(win, null), null);
});

test("pickFile falls back to a file input", async () => {
  const listeners = {};
  const file = { name: "a.docx" };
  const input = {
    files: [file],
    addEventListener: (type, fn) => (listeners[type] = fn),
    click: () => listeners.change(),
  };
  const doc = { createElement: (tag) => (assert.equal(tag, "input"), input) };
  const picked = await pickFile({}, doc);
  assert.deepEqual(picked, { file, handle: null });
  assert.equal(input.type, "file");
  assert.match(input.accept, /\.docx/);
});

test("pickSaveLocation suggests the current name and handles cancel", async () => {
  let options;
  const handle = { name: "copy.docx" };
  const win = { showSaveFilePicker: async (o) => ((options = o), handle) };
  assert.equal(await pickSaveLocation("report.docx", win), handle);
  assert.equal(options.suggestedName, "report.docx");
  const cancelling = { showSaveFilePicker: async () => Promise.reject(abortError()) };
  assert.equal(await pickSaveLocation("x.docx", cancelling), null);
});

test("writeToHandle writes and closes, aborting on failure", async () => {
  const calls = [];
  const writable = {
    write: async (blob) => calls.push(["write", blob]),
    close: async () => calls.push(["close"]),
    abort: async () => calls.push(["abort"]),
  };
  const blob = new Blob(["x"]);
  await writeToHandle({ createWritable: async () => writable }, blob);
  assert.deepEqual(calls, [["write", blob], ["close"]]);

  calls.length = 0;
  writable.write = async () => {
    throw new Error("disk full");
  };
  await assert.rejects(writeToHandle({ createWritable: async () => writable }, blob), /disk full/);
  assert.deepEqual(calls, [["abort"]]);
});

test("downloadBlob clicks a temporary download link", () => {
  const events = [];
  const link = { click: () => events.push("click"), remove: () => events.push("remove") };
  const doc = { createElement: () => link, body: { appendChild: () => events.push("append") } };
  const url = { createObjectURL: () => "blob:local/1", revokeObjectURL: () => {} };
  downloadBlob(new Blob(["x"]), "report.docx", doc, url);
  assert.equal(link.href, "blob:local/1");
  assert.equal(link.download, "report.docx");
  assert.deepEqual(events, ["append", "click", "remove"]);
});
