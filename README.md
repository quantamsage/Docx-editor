# DOCX Editor

A local, web-based editor for Microsoft Word `.docx` documents. A Rust (Axum/Tokio) server runs on your computer and serves a Word-style editing interface built with plain HTML, CSS and JavaScript. Nothing is sent over the internet.

> **Status: Phase 1 (project foundation).** The local server, the application interface and the communication between them work. Opening, editing and saving documents arrive in later phases. Toolbar controls for those features are shown but disabled until then.

## Requirements

- [Rust](https://rustup.rs/) 1.85 or newer (edition 2024).
- Optional, only for the frontend tests: [Node.js](https://nodejs.org/) 20 or newer. The application itself does not use Node.

## Running

```sh
cargo run --release
```

The editor starts at <http://localhost:8080> and opens in your default browser. Press `Ctrl+C` to stop it.

Options:

| Option | Description |
| --- | --- |
| `-p`, `--port <PORT>` | Port to listen on (default `8080`; `0` picks any free port). Also settable with `DOCX_EDITOR_PORT`. |
| `--no-open` | Don't open the browser automatically. |
| `-h`, `--help` | Show help. |
| `-V`, `--version` | Show the version. |

Set `RUST_LOG=debug` for more detailed logs.

The server only listens on `127.0.0.1`, so other computers on the network cannot reach it. A built binary is self-contained: the interface is embedded in it.

```sh
cargo build --release
./target/release/docx-editor
```

## Tests

```sh
cargo test                     # Rust unit and HTTP integration tests
cargo clippy --all-targets     # lints
cargo fmt --check              # formatting
npm test                       # frontend tests (Node's built-in test runner; no packages to install)
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for how the application is organized.
