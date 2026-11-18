// Client for the local editor server's JSON API.
//
// Every response is JSON. Errors have the shape
// {"error": {"code": "...", "message": "..."}} and are raised as ApiError.

export const DEFAULT_TIMEOUT_MS = 5000;

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
 * @param {number} [options.timeoutMs]
 * @param {typeof fetch} [options.fetchImpl]  Injected in tests.
 * @returns {Promise<any>} The parsed JSON body.
 */
export async function request(path, { method = "GET", body, timeoutMs = DEFAULT_TIMEOUT_MS, fetchImpl = globalThis.fetch } = {}) {
  if (!path.startsWith("/api/")) {
    throw new ApiError(`Refusing to call non-API path ${path}`, { code: "invalid_path" });
  }
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), timeoutMs);
  const init = { method, headers: { Accept: "application/json" }, signal: controller.signal };
  if (body !== undefined) {
    init.headers["Content-Type"] = "application/json";
    init.body = JSON.stringify(body);
  }

  let response;
  try {
    response = await fetchImpl(path, init);
  } catch (err) {
    if (err && err.name === "AbortError") {
      throw new ApiError("The local server did not respond in time.", { code: "timeout" });
    }
    throw new ApiError("Cannot reach the local editor server. Is it still running?", { code: "network" });
  } finally {
    clearTimeout(timer);
  }

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
    throw new ApiError(
      (error && error.message) || `The server returned HTTP ${response.status}.`,
      { status: response.status, code: (error && error.code) || "http_error" },
    );
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
