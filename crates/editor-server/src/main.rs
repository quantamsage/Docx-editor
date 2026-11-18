use std::io::ErrorKind;
use std::process::ExitCode;

use docx_editor::config::{self, Command, Config};
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

fn main() -> ExitCode {
    let env_port = std::env::var(config::PORT_ENV).ok();
    let config = match config::parse_args(std::env::args().skip(1), env_port.as_deref()) {
        Ok(Command::Run(config)) => config,
        Ok(Command::Help) => {
            println!("{}", config::USAGE);
            return ExitCode::SUCCESS;
        }
        Ok(Command::Version) => {
            println!("docx-editor {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(2);
        }
    };

    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(err) => {
            eprintln!("error: could not start the async runtime: {err}");
            return ExitCode::FAILURE;
        }
    };
    runtime.block_on(run(config))
}

async fn run(config: Config) -> ExitCode {
    let listener = match TcpListener::bind(config.bind_addr()).await {
        Ok(listener) => listener,
        Err(err) if err.kind() == ErrorKind::AddrInUse => {
            eprintln!(
                "error: port {} is already in use. Start with --port <PORT> (or --port 0 for any free port).",
                config.port
            );
            return ExitCode::FAILURE;
        }
        Err(err) => {
            eprintln!("error: could not listen on {}: {err}", config.bind_addr());
            return ExitCode::FAILURE;
        }
    };
    let port = match listener.local_addr() {
        Ok(addr) => addr.port(),
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::FAILURE;
        }
    };
    let url = format!("http://localhost:{port}");
    tracing::info!("DOCX editor running at {url} (press Ctrl+C to stop)");

    if config.open_browser {
        open_browser(&url);
    }

    match docx_editor::serve(listener, shutdown_signal()).await {
        Ok(()) => {
            tracing::info!("server stopped");
            ExitCode::SUCCESS
        }
        Err(err) => {
            tracing::error!("server error: {err}");
            ExitCode::FAILURE
        }
    }
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        tracing::error!("could not listen for Ctrl+C: {err}");
        std::future::pending::<()>().await;
    }
    tracing::info!("shutting down");
}

/// Opens `url` in the default browser using the platform's launcher. Failure
/// is not fatal: the URL is printed so the user can open it manually.
fn open_browser(url: &str) {
    let result = if cfg!(target_os = "windows") {
        std::process::Command::new("cmd").args(["/C", "start", "", url]).spawn()
    } else if cfg!(target_os = "macos") {
        std::process::Command::new("open").arg(url).spawn()
    } else {
        std::process::Command::new("xdg-open").arg(url).spawn()
    };
    if let Err(err) = result {
        tracing::warn!("could not open a browser ({err}); open {url} manually");
    }
}
