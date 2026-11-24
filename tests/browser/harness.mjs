// Shared plumbing for the browser tests: starts the built editor and a
// headless Chromium-based browser (Edge or Chrome), and talks to the page
// through the DevTools protocol with Node's built-in WebSocket.

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

export const ROOT = fileURLToPath(new URL("../../", import.meta.url));

export const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

export function findBrowser() {
  const candidates = [
    process.env.BROWSER,
    "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
    "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
    "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
    "/usr/bin/google-chrome",
    "/usr/bin/chromium",
    "/usr/bin/microsoft-edge",
  ].filter(Boolean);
  const found = candidates.find((path) => existsSync(path));
  if (!found) throw new Error("No Chromium-based browser found; set BROWSER to its path.");
  return found;
}

/** Starts the built editor on a free port: {server, url: Promise<string>}. */
export function startServer() {
  const exe = process.platform === "win32" ? "docx-editor.exe" : "docx-editor";
  const binary = ["release", "debug"].map((p) => join(ROOT, "target", p, exe)).find((p) => existsSync(p));
  if (!binary) throw new Error("Build the editor first (cargo build --release).");
  const server = spawn(binary, ["--no-open", "--port", "0"], { stdio: ["ignore", "pipe", "pipe"], env: { ...process.env, NO_COLOR: "1" } });
  const url = new Promise((resolve, reject) => {
    let output = "";
    const onData = (chunk) => {
      output += chunk;
      const match = /running at (http:\/\/localhost:\d+)/.exec(output.replace(/\u001b\[[0-9;]*m/g, ""));
      if (match) resolve(match[1]);
    };
    server.stdout.on("data", onData);
    server.stderr.on("data", onData);
    server.on("exit", (code) => reject(new Error(`server exited (${code}): ${output}`)));
  });
  return { server, url };
}

async function connect(port) {
  for (let attempt = 0; attempt < 50; attempt++) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
      const page = targets.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      // Browser still starting.
    }
    await sleep(200);
  }
  throw new Error("Could not connect to the browser's DevTools endpoint.");
}

export function cdpClient(url) {
  const socket = new WebSocket(url);
  let nextId = 1;
  const pending = new Map();
  const listeners = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
    if (message.method && listeners.has(message.method)) listeners.get(message.method)(message.params);
    if (message.id && pending.has(message.id)) {
      const { resolve, reject } = pending.get(message.id);
      pending.delete(message.id);
      if (message.error) reject(new Error(message.error.message));
      else resolve(message.result);
    }
  });
  const ready = new Promise((resolve, reject) => {
    socket.addEventListener("open", resolve);
    socket.addEventListener("error", reject);
  });
  return {
    ready,
    send(method, params = {}) {
      const id = nextId++;
      socket.send(JSON.stringify({ id, method, params }));
      return new Promise((resolve, reject) => pending.set(id, { resolve, reject }));
    },
    /** Calls `fn` with the parameters of each `method` event. */
    on: (method, fn) => listeners.set(method, fn),
    close: () => socket.close(),
  };
}

/**
 * Starts the browser (and, without `appUrl`, the editor), connects to the
 * page and returns helpers. Call `close()` when done.
 */
export async function launch({ port, appUrl = null, name = "browser" }) {
  const profile = mkdtempSync(join(tmpdir(), `docx-editor-${name}-`));
  const browser = spawn(
    findBrowser(),
    ["--headless=new", "--disable-gpu", "--no-first-run", `--remote-debugging-port=${port}`, `--user-data-dir=${profile}`, "about:blank"],
    { stdio: "ignore" },
  );
  const started = appUrl ? null : startServer();
  async function close() {
    const exited = new Promise((resolve) => browser.once("exit", resolve));
    browser.kill();
    started?.server.kill();
    await Promise.race([exited, sleep(5000)]);
    try {
      rmSync(profile, { recursive: true, force: true, maxRetries: 10, retryDelay: 300 });
    } catch {
      // The browser may still hold files briefly; the OS cleans its temp folder.
    }
  }
  try {
    const url = appUrl || (await started.url);
    const cdp = cdpClient(await connect(port));
    await cdp.ready;
    await cdp.send("Runtime.enable");
    const evaluate = async (expression) => {
      const { result, exceptionDetails } = await cdp.send("Runtime.evaluate", { expression, awaitPromise: true, returnByValue: true });
      if (exceptionDetails) throw new Error(exceptionDetails.exception?.description || exceptionDetails.text);
      return result.value;
    };
    const waitFor = async (expression, timeoutMs = 10000) => {
      const deadline = Date.now() + timeoutMs;
      while (Date.now() < deadline) {
        if (await evaluate(expression)) return true;
        await sleep(100);
      }
      return false;
    };
    /** A real key press (rawKeyDown, char for Enter, keyUp). `modifiers`: 1 Alt, 2 Ctrl, 4 Meta, 8 Shift. */
    const press = async (key, code, keyCode, modifiers = 0) => {
      await cdp.send("Input.dispatchKeyEvent", { type: "rawKeyDown", key, code, windowsVirtualKeyCode: keyCode, modifiers });
      if (key === "Enter" && !(modifiers & 2)) await cdp.send("Input.dispatchKeyEvent", { type: "char", key, code, text: "\r", windowsVirtualKeyCode: keyCode, modifiers });
      await cdp.send("Input.dispatchKeyEvent", { type: "keyUp", key, code, windowsVirtualKeyCode: keyCode, modifiers });
    };
    const type = (value) => cdp.send("Input.insertText", { text: value });
    /** Drops a file (base64 bytes) on the workspace, as "Open" would load it. */
    const drop = (base64, fileName) =>
      evaluate(`(() => {
        const bytes = Uint8Array.from(atob(${JSON.stringify(base64)}), (c) => c.charCodeAt(0));
        const transfer = new DataTransfer();
        transfer.items.add(new File([bytes], ${JSON.stringify(fileName)}));
        const target = document.getElementById("workspace");
        target.dispatchEvent(new DragEvent("dragover", { dataTransfer: transfer, bubbles: true, cancelable: true }));
        target.dispatchEvent(new DragEvent("drop", { dataTransfer: transfer, bubbles: true, cancelable: true }));
      })()`);
    return { cdp, url, evaluate, waitFor, press, type, drop, close, server: started?.server ?? null };
  } catch (err) {
    await close();
    throw err;
  }
}

/** A check list that prints each result and remembers failures. */
export function checker() {
  const failures = [];
  const check = (condition, message) => {
    console.log(`${condition ? "ok  " : "FAIL"} ${message}`);
    if (!condition) failures.push(message);
  };
  return { check, failures };
}
