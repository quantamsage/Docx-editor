// Browser smoke test: drives the real interface in a headless Chromium-based
// browser (Edge or Chrome) through the DevTools protocol.
//
// Usage:
//   cargo build --release
//   node tests/browser/smoke.mjs            # starts its own server on a free port
//   node tests/browser/smoke.mjs <url>      # or test an already running editor
//
// Set BROWSER to the browser executable if it is not found automatically,
// and SCREENSHOT=<file.png> to save a screenshot of the opened document.
// Native file pickers cannot be automated, so files are opened by dropping
// them onto the workspace, which uses the same code path as "Open".

import { spawn } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const APP_URL_ARG = process.argv[2];
const ROOT = fileURLToPath(new URL("../../", import.meta.url));
const FIXTURE = fileURLToPath(new URL("../fixtures/word-sample.docx", import.meta.url));
const PORT = 9333;

function findBrowser() {
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

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** Starts the built editor on a free port and resolves with its URL. */
function startServer() {
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

async function connect() {
  for (let attempt = 0; attempt < 50; attempt++) {
    try {
      const targets = await (await fetch(`http://127.0.0.1:${PORT}/json/list`)).json();
      const page = targets.find((t) => t.type === "page");
      if (page) return page.webSocketDebuggerUrl;
    } catch {
      // Browser still starting.
    }
    await sleep(200);
  }
  throw new Error("Could not connect to the browser's DevTools endpoint.");
}

function cdpClient(url) {
  const socket = new WebSocket(url);
  let nextId = 1;
  const pending = new Map();
  socket.addEventListener("message", (event) => {
    const message = JSON.parse(event.data);
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
    close: () => socket.close(),
  };
}

async function main() {
  const profile = mkdtempSync(join(tmpdir(), "docx-editor-smoke-"));
  const browser = spawn(
    findBrowser(),
    ["--headless=new", "--disable-gpu", "--no-first-run", `--remote-debugging-port=${PORT}`, `--user-data-dir=${profile}`, "about:blank"],
    { stdio: "ignore" },
  );
  const started = APP_URL_ARG ? null : startServer();
  const APP_URL = APP_URL_ARG || (await started.url);
  const failures = [];
  const check = (condition, message) => {
    console.log(`${condition ? "ok  " : "FAIL"} ${message}`);
    if (!condition) failures.push(message);
  };

  try {
    const cdp = cdpClient(await connect());
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
    const text = (id) => evaluate(`document.getElementById(${JSON.stringify(id)}).textContent.trim()`);
    const drop = (base64, name) =>
      evaluate(`(() => {
        const bytes = Uint8Array.from(atob(${JSON.stringify(base64)}), (c) => c.charCodeAt(0));
        const transfer = new DataTransfer();
        transfer.items.add(new File([bytes], ${JSON.stringify(name)}));
        const target = document.getElementById("workspace");
        target.dispatchEvent(new DragEvent("dragover", { dataTransfer: transfer, bubbles: true, cancelable: true }));
        target.dispatchEvent(new DragEvent("drop", { dataTransfer: transfer, bubbles: true, cancelable: true }));
      })()`);

    await cdp.send("Page.navigate", { url: APP_URL });
    check(await waitFor(`document.getElementById("status-connection")?.dataset.state === "connected"`), "connects to the local server");
    check(await evaluate(`document.querySelector('[data-action="save"]').disabled`), "Save is disabled with no document");
    check(await evaluate(`getComputedStyle(document.getElementById("notice")).display === "none"`), "notice bar is hidden at start");

    // New document.
    await evaluate(`document.querySelector('[data-action="new"]').click()`);
    check(await waitFor(`!document.getElementById("doc-summary").hidden`), "New shows a document");
    check((await text("document-name")) === "Untitled document.docx", "new document is untitled");
    check((await text("save-state")) === "Not saved yet", "new document is marked not saved");
    check((await text("summary-layout")) === "Page layout: A4, portrait", "blank document is A4 portrait");
    check(!(await evaluate(`document.querySelector('[data-action="save"]').disabled`)), "Save is enabled");

    // Open the Word fixture by dropping it.
    await drop(readFileSync(FIXTURE).toString("base64"), "word-sample.docx");
    check(await waitFor(`document.getElementById("document-name").textContent === "word-sample.docx"`), "dropping a .docx opens it");
    check((await text("save-state")) === "Opened", "opened document state");
    check(await evaluate(`getComputedStyle(document.getElementById("notice")).display === "none"`), "no notice after a successful open");
    const stats = await evaluate(`[...document.querySelectorAll("#summary-stats li")].map((li) => li.textContent)`);
    check(stats.includes("1 table") && stats.includes("1 image") && stats.includes("1 hyperlink"), `summary lists content (${stats.join(", ")})`);
    const headings = await evaluate(`[...document.querySelectorAll("#outline li")].map((li) => li.textContent)`);
    check(headings[0] === "Quarterly Report" && headings.includes("Appendix"), `outline lists headings (${headings.join(" / ")})`);
    check((await text("status-words")).endsWith("words") && (await text("status-words")) !== "0 words", "status bar shows the word count");

    if (process.env.SCREENSHOT) {
      await cdp.send("Emulation.setDeviceMetricsOverride", { width: 1280, height: 900, deviceScaleFactor: 1, mobile: false });
      const { data } = await cdp.send("Page.captureScreenshot", { format: "png" });
      writeFileSync(process.env.SCREENSHOT, Buffer.from(data, "base64"));
      console.log(`     screenshot saved to ${process.env.SCREENSHOT}`);
    }

    // An invalid file shows an error and keeps the open document.
    await drop(Buffer.from("not a word file").toString("base64"), "broken.docx");
    check(await waitFor(`!document.getElementById("notice").hidden`), "invalid file shows a notice");
    check((await text("notice-text")).includes("not a valid .docx"), "notice explains the problem");
    check((await text("document-name")) === "word-sample.docx", "the open document stays open");

    // A file that is not a Word document is refused before upload.
    await drop(Buffer.from("hello").toString("base64"), "notes.txt");
    check((await text("notice-text")).includes("is not a Word document"), "non-.docx drops are refused");

    // The open document can be downloaded through the same endpoint Save uses.
    const size = await evaluate(`(async () => {
      const res = await fetch("/api/documents/" + 2 + "/download");
      return res.ok ? (await res.arrayBuffer()).byteLength : -res.status;
    })()`);
    check(size > 20000, `saved document is returned (${size} bytes)`);

    cdp.close();
  } finally {
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

  if (failures.length) {
    console.error(`\n${failures.length} check(s) failed.`);
    process.exit(1);
  }
  console.log("\nAll browser checks passed.");
}

main().catch((err) => {
  console.error(err);
  process.exit(1);
});
