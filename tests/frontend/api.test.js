import { test } from "node:test";
import assert from "node:assert/strict";

import {
  ApiError,
  applyEdits,
  closeDocument,
  downloadDocument,
  getDocument,
  getHealth,
  mediaUrl,
  newDocument,
  openDocument,
  request,
} from "../../web/js/api.js";

/** Builds a fake fetch that records calls and returns the given response. */
function fakeFetch(respond) {
  const calls = [];
  const impl = async (url, init) => {
    calls.push({ url, init });
    return respond(url, init);
  };
  impl.calls = calls;
  return impl;
}

function jsonResponse(status, body) {
  return new Response(body === undefined ? "" : JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}

test("getHealth returns the parsed health status", async () => {
  const fetchImpl = fakeFetch(() => jsonResponse(200, { status: "ok", app: "docx-editor", version: "0.1.0" }));
  const health = await getHealth({ fetchImpl });
  assert.deepEqual(health, { status: "ok", app: "docx-editor", version: "0.1.0" });
  assert.equal(fetchImpl.calls[0].url, "/api/health");
  assert.equal(fetchImpl.calls[0].init.method, "GET");
  assert.equal(fetchImpl.calls[0].init.headers.Accept, "application/json");
});

test("getHealth rejects an unexpected payload", async () => {
  const fetchImpl = fakeFetch(() => jsonResponse(200, { status: "degraded" }));
  await assert.rejects(getHealth({ fetchImpl }), (err) => err instanceof ApiError && err.code === "invalid_response");
});

test("request sends JSON bodies", async () => {
  const fetchImpl = fakeFetch(() => jsonResponse(200, { saved: true }));
  const data = await request("/api/documents", { method: "POST", body: { name: "a.docx" }, fetchImpl });
  assert.deepEqual(data, { saved: true });
  const { init } = fetchImpl.calls[0];
  assert.equal(init.method, "POST");
  assert.equal(init.headers["Content-Type"], "application/json");
  assert.equal(init.body, '{"name":"a.docx"}');
});

test("server errors become ApiError with the server's code and message", async () => {
  const fetchImpl = fakeFetch(() => jsonResponse(404, { error: { code: "not_found", message: "No such API endpoint." } }));
  await assert.rejects(request("/api/nope", { fetchImpl }), (err) => {
    assert.ok(err instanceof ApiError);
    assert.equal(err.status, 404);
    assert.equal(err.code, "not_found");
    assert.equal(err.message, "No such API endpoint.");
    return true;
  });
});

test("errors without a JSON body still produce a readable message", async () => {
  const fetchImpl = fakeFetch(() => new Response("", { status: 502 }));
  await assert.rejects(request("/api/health", { fetchImpl }), { name: "ApiError", status: 502, code: "http_error" });
});

test("non-JSON responses are reported as invalid", async () => {
  const fetchImpl = fakeFetch(() => new Response("<html>oops</html>", { status: 200 }));
  await assert.rejects(request("/api/health", { fetchImpl }), { code: "invalid_response" });
});

test("network failures are reported as the server being unreachable", async () => {
  const fetchImpl = async () => {
    throw new TypeError("Failed to fetch");
  };
  await assert.rejects(request("/api/health", { fetchImpl }), (err) => {
    assert.equal(err.code, "network");
    assert.match(err.message, /local editor server/);
    return true;
  });
});

test("slow responses time out", async () => {
  const fetchImpl = (url, init) =>
    new Promise((_, reject) => {
      init.signal.addEventListener("abort", () => reject(new DOMException("aborted", "AbortError")));
    });
  await assert.rejects(request("/api/health", { fetchImpl, timeoutMs: 10 }), { code: "timeout" });
});

test("only API paths may be requested", async () => {
  const fetchImpl = fakeFetch(() => jsonResponse(200, {}));
  for (const path of ["https://example.com/api/health", "/static/js/app.js", "//evil.example/api/x"]) {
    await assert.rejects(request(path, { fetchImpl }), { code: "invalid_path" });
  }
  assert.equal(fetchImpl.calls.length, 0);
});

test("newDocument posts to the create endpoint", async () => {
  const info = { id: 1, name: "Untitled document.docx", modified: false, model: { blocks: [] } };
  const fetchImpl = fakeFetch(() => jsonResponse(201, info));
  assert.deepEqual(await newDocument({ fetchImpl }), info);
  assert.equal(fetchImpl.calls[0].url, "/api/documents/new");
  assert.equal(fetchImpl.calls[0].init.method, "POST");
});

test("openDocument sends the raw file with its name", async () => {
  const fetchImpl = fakeFetch(() => jsonResponse(201, { id: 2, name: "Q3 report & notes.docx" }));
  const file = new Blob(["PK fake"]);
  const info = await openDocument(file, "Q3 report & notes.docx", { fetchImpl });
  assert.equal(info.id, 2);
  const { url, init } = fetchImpl.calls[0];
  assert.equal(url, "/api/documents/open?name=Q3%20report%20%26%20notes.docx");
  assert.equal(init.method, "POST");
  assert.equal(init.body, file);
  assert.equal(init.headers["Content-Type"], "application/octet-stream");
});

test("openDocument surfaces the server's explanation", async () => {
  const fetchImpl = fakeFetch(() =>
    jsonResponse(422, { error: { code: "unsupported_document", message: "The file is password-protected or uses the old .doc format." } }),
  );
  await assert.rejects(openDocument(new Blob(["x"]), "old.doc", { fetchImpl }), {
    status: 422,
    code: "unsupported_document",
    message: "The file is password-protected or uses the old .doc format.",
  });
});

test("downloadDocument returns a Blob and can rename", async () => {
  const fetchImpl = fakeFetch(() => new Response(new Uint8Array([0x50, 0x4b, 3, 4]), { status: 200 }));
  const blob = await downloadDocument(7, undefined, { fetchImpl });
  assert.ok(blob instanceof Blob);
  assert.deepEqual([...new Uint8Array(await blob.arrayBuffer())], [0x50, 0x4b, 3, 4]);
  assert.equal(fetchImpl.calls[0].url, "/api/documents/7/download");

  await downloadDocument(7, "New name.docx", { fetchImpl });
  assert.equal(fetchImpl.calls[1].url, "/api/documents/7/download?name=New%20name.docx");
});

test("downloadDocument errors are parsed as JSON", async () => {
  const fetchImpl = fakeFetch(() => jsonResponse(404, { error: { code: "not_found", message: "That document is not open." } }));
  await assert.rejects(downloadDocument(9, undefined, { fetchImpl }), { code: "not_found", status: 404 });
});

test("getDocument and closeDocument", async () => {
  const fetchImpl = fakeFetch((url, init) => (init.method === "DELETE" ? new Response(null, { status: 204 }) : jsonResponse(200, { id: 3 })));
  assert.deepEqual(await getDocument(3, { fetchImpl }), { id: 3 });
  await closeDocument(3, { fetchImpl });
  assert.equal(fetchImpl.calls[1].url, "/api/documents/3");
  assert.equal(fetchImpl.calls[1].init.method, "DELETE");
});

test("applyEdits posts the base revision and edits as JSON", async () => {
  const fetchImpl = fakeFetch(() => jsonResponse(200, { id: 3, revision: 5, model: { blocks: [] } }));
  const edits = [{ op: "splice", first: 0, last: 0, content: [{ type: "text", text: "Hi" }] }];
  const result = await applyEdits(3, 4, edits, { fetchImpl });
  assert.equal(result.revision, 5);
  const { url, init } = fetchImpl.calls[0];
  assert.equal(url, "/api/documents/3/edits");
  assert.equal(init.method, "POST");
  assert.deepEqual(JSON.parse(init.body), { base_revision: 4, edits });
});

test("applyEdits surfaces stale revisions", async () => {
  const fetchImpl = fakeFetch(() => jsonResponse(409, { error: { code: "stale_revision", message: "The document changed." } }));
  await assert.rejects(applyEdits(3, 1, [], { fetchImpl }), (err) => err instanceof ApiError && err.code === "stale_revision" && err.status === 409);
});

test("mediaUrl encodes each part segment", () => {
  assert.equal(mediaUrl(2, "word/media/image 1.png"), "/api/documents/2/media/word/media/image%201.png");
  assert.equal(mediaUrl(2, "word/media/a#b.png"), "/api/documents/2/media/word/media/a%23b.png");
});
