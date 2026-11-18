# Architecture

## Overview

```
 Browser (HTML/CSS/vanilla JS)  ──HTTP/JSON on 127.0.0.1──▶  editor-server (Axum/Tokio)  ──Rust calls──▶  docx-engine (Phase 2+)
```

- **Frontend** (`web/`): the editing interface. Plain ES modules, no framework and no build step. All logic that can be separated from the DOM lives in small modules that are unit-tested with Node's test runner.
- **Backend** (`crates/editor-server`): a thin HTTP layer. It serves the embedded frontend, exposes a JSON API under `/api`, and (from Phase 2) handles file operations and application state by calling the document engine.
- **Document engine** (`crates/docx-engine`, added in Phase 2): DOCX parsing, the in-memory document model, editing operations and serialization. It is a plain library with no HTTP or Tokio dependency, so it can be tested and reasoned about on its own.
- **File management**: opening and saving go through the browser (file picker and download/save dialogs). The server only touches files the user explicitly selects or saves.

## Layout

```
Cargo.toml                 workspace (shared dependency versions)
crates/editor-server/
  src/main.rs              CLI entry: parse args → bind 127.0.0.1:<port> → open browser → serve until Ctrl+C
  src/lib.rs               serve(listener, shutdown)
  src/config.rs            command-line / environment configuration
  src/app.rs               router: page, static assets, /api routes, fallbacks
  src/assets.rs            frontend files embedded with include_bytes!
  src/security.rs          loopback Host check, cross-origin check, security headers
  src/error.rs             ApiError → JSON error responses
  tests/http.rs            router and live-server integration tests
web/
  index.html               application shell (toolbars, page, status bar)
  css/app.css              Word-style layout and paper-like pages
  js/app.js                entry point: wires the DOM to the modules below
  js/api.js                fetch wrapper for the JSON API (timeouts, error mapping)
  js/status.js             status-bar logic: word count, page position, connection monitor
tests/frontend/            Node test-runner tests for web/js modules
package.json               only defines `npm test`; no dependencies
```

## Frontend ↔ backend communication

- The page and its assets are served from the binary. Assets are embedded at compile time, so the executable is self-contained, and changing a frontend file requires a rebuild.
- The frontend calls JSON endpoints under `/api/` through `api.js`. Successful responses are JSON; errors are `{"error": {"code", "message"}}` with an HTTP status. `api.js` turns those, network failures and timeouts into `ApiError`s with a user-readable message.
- Phase 1 endpoint: `GET /api/health` returns `{"status": "ok", "app": "docx-editor", "version": "…"}`. The status bar polls it (every 15 s while connected, every 3 s while offline, and immediately when the window regains focus) and shows whether the local server is reachable.
- Later phases add document endpoints on the same pattern (for example, upload a `.docx` → the engine parses it → the frontend receives a JSON document model; edits are sent back as operations; save returns the serialized `.docx`).

## Local-only security

The editor runs on the user's machine, but any web page the user visits can try to send requests to `localhost`. From Phase 1 on:

- The server binds to `127.0.0.1` only.
- **Host check:** requests whose `Host` header is not `localhost`, `127.0.0.1` or `[::1]` are rejected with 403. This defeats DNS-rebinding attacks, where an attacker's domain resolves to 127.0.0.1.
- **Origin check:** state-changing requests (anything except `GET`/`HEAD`/`OPTIONS`) must come from a loopback origin. Requests with a foreign `Origin` or `Sec-Fetch-Site: cross-site` are rejected. Clients that send neither (such as `curl`) are allowed, since they already run on the machine.
- **Headers:** a strict Content-Security-Policy (`default-src 'self'`; no inline scripts or styles; nothing from the network), `nosniff`, `X-Frame-Options: DENY`, `Referrer-Policy: no-referrer` and same-origin COOP/CORP.
- No telemetry, no external requests, no accounts.

## Decisions

- **Workspace with separate crates.** The server and the document engine are separate crates, so the engine can't accidentally depend on HTTP types.
- **Embedded assets via `include_bytes!`**, with no static-file or embedding crate. This gives one self-contained binary and an explicit list of served files, so nothing on disk can be served by accident.
- **No CLI parsing crate.** There are only four options.
- **Hand-rolled browser launch** (`start` / `open` / `xdg-open`). Failing to launch a browser is not an error; the URL is printed.
- **A port conflict is a clear error** rather than silently picking another port, so the URL the user expects stays stable. `--port 0` picks a free port explicitly.
- **Frontend tests use Node's built-in runner** without npm packages. DOM-dependent wiring (`app.js`) is kept minimal and is covered by the HTTP tests (every referenced asset is served, no inline scripts) and by loading the page in a real browser.
