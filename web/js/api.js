// Client for the local editor server's JSON API.
//
// Responses are JSON unless a binary response is requested. Errors have the
// shape {"error": {"code": "...", "message": "..."}} and are raised as ApiError.

export const DEFAULT_TIMEOUT_MS = 5000;
/** Opening and saving large documents can take longer than a health check. */
export const DOCUMENT_TIMEOUT_MS = 120000;

export class ApiError extends Error {
  /**
   * @param {string} message  Human-readable description, safe to display.
   * @param {{status?: number, code?: string}} [details]
   */
  constructor(message, { status = 0, code = "error" } = {}) {
    super(message);
    this.name = "ApiError";
    this.status = status;
    this.code = code;
  }
}

/**
 * Performs a request against the local API.
 *
 * @param {string} path  Path below the server root, e.g. "/api/health".
 * @param {object} [options]
 * @param {string} [options.method]
 * @param {unknown} [options.body]  Serialized as JSON when present.
 * @param {Blob|ArrayBuffer} [options.rawBody]  Sent as-is (application/octet-stream).
 * @param {"json"|"blob"} [options.responseType]
 * @param {number} [options.timeoutMs]
 * @param {typeof fetch} [options.fetchImpl]  Injected in tests.
 * @returns {Promise<any>} The parsed JSON body, or a Blob.
 */
export async function request(
  path,
  { method = "GET", body, rawBody, responseType = "json", timeoutMs = DEFAULT_TIMEOUT_MS, fetchImpl = globalThis.fetch } = {},
) {
  if (!path.startsWith("/api/")) {
    throw new ApiError(`Refusing to call non-API path ${path}`, { code: "invalid_path" });
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  const init = { method, headers: { Accept: responseType === "blob" ? "*/*" : "application/json" }, signal: controller.signal };
  if (rawBody !== undefined) {
    init.headers["Content-Type"] = "application/octet-stream";
    init.body = rawBody;
  } else if (body !== undefined) {
    init.headers["Content-Type"] = "application/json";
    init.body = JSON.stringify(body);
  }

  let response;
  try {
    response = await fetchImpl(path, init);
    if (response.ok && responseType === "blob") {
      return await response.blob();
    }
    return await parseJson(response);
  } catch (err) {
    if (err instanceof ApiError) throw err;
    if (err && err.name === "AbortError") {
      throw new ApiError("The local server did not respond in time.", { code: "timeout" });
    }
    throw new ApiError("Cannot reach the local editor server. Is it still running?", { code: "network" });
  } finally {
    clearTimeout(timer);
  }
}

async function parseJson(response) {
  let data = null;
  const text = await response.text();
  if (text) {
    try {
      data = JSON.parse(text);
    } catch {
      throw new ApiError(`The server sent an unreadable response (HTTP ${response.status}).`, {
        status: response.status,
        code: "invalid_response",
      });
    }
  }
  if (!response.ok) {
    const error = data && data.error;
    throw new ApiError((error && error.message) || `The server returned HTTP ${response.status}.`, {
      status: response.status,
      code: (error && error.code) || "http_error",
    });
  }
  return data;
}

/**
 * Checks that the local server is running.
 * @returns {Promise<{status: string, app: string, version: string}>}
 */
export async function getHealth(options) {
  const data = await request("/api/health", options);
  if (!data || data.status !== "ok" || typeof data.version !== "string") {
    throw new ApiError("The server reported an unexpected health status.", { code: "invalid_response" });
  }
  return data;
}

/**
 * Document as returned by the server.
 * @typedef {{id: number, name: string, modified: boolean, model: object}} DocumentInfo
 */

/** Creates a blank document. @returns {Promise<DocumentInfo>} */
export function newDocument(options = {}) {
  return request("/api/documents/new", { ...options, method: "POST", timeoutMs: DOCUMENT_TIMEOUT_MS });
}

/**
 * Sends the bytes of a .docx file to the server to open it.
 * @param {Blob} file
 * @param {string} name  File name shown to the user.
 * @returns {Promise<DocumentInfo>}
 */
export function openDocument(file, name, options = {}) {
  return request(`/api/documents/open?name=${encodeURIComponent(name)}`, {
    ...options,
    method: "POST",
    rawBody: file,
    timeoutMs: DOCUMENT_TIMEOUT_MS,
  });
}

/** @returns {Promise<DocumentInfo>} */
export function getDocument(id, options = {}) {
  return request(`/api/documents/${encodeURIComponent(id)}`, options);
}

/**
 * Serializes the document and returns the .docx file. Passing `name`
 * renames the open document (used by "Save as").
 * @returns {Promise<Blob>}
 */
export function downloadDocument(id, name, options = {}) {
  const query = name ? `?name=${encodeURIComponent(name)}` : "";
  return request(`/api/documents/${encodeURIComponent(id)}/download${query}`, {
    ...options,
    responseType: "blob",
    timeoutMs: DOCUMENT_TIMEOUT_MS,
  });
}

/** Closes a document on the server. */
export async function closeDocument(id, options = {}) {
  await request(`/api/documents/${encodeURIComponent(id)}`, { ...options, method: "DELETE" });
}
