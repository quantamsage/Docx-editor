import { test } from "node:test";
import assert from "node:assert/strict";

import { ApiError, getHealth, request } from "../../web/js/api.js";

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
