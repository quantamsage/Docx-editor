//! Command-line configuration.
//!
//! The server always binds to the loopback interface: the editor is a local
//! application and must not be reachable from other machines.

use std::fmt;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};

/// Default port, overridable with `--port` or `DOCX_EDITOR_PORT`.
pub const DEFAULT_PORT: u16 = 8080;

/// Environment variable that sets the port when `--port` is not given.
pub const PORT_ENV: &str = "DOCX_EDITOR_PORT";

/// Runtime configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// TCP port on 127.0.0.1. `0` lets the OS pick a free port.
    pub port: u16,
    /// Open the editor in the default browser after starting.
    pub open_browser: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self { port: DEFAULT_PORT, open_browser: true }
    }
}

impl Config {
    /// The socket address the server listens on.
    pub fn bind_addr(&self) -> SocketAddr {
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), self.port)
    }
}

/// What the program should do, as decided by the command line.
#[derive(Debug, PartialEq, Eq)]
pub enum Command {
    Run(Config),
    Help,
    Version,
}

/// Invalid command-line input.
#[derive(Debug, PartialEq, Eq)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ConfigError {}

/// Usage text for `--help`.
pub const USAGE: &str = "\
Usage: docx-editor [OPTIONS]

Starts the local DOCX editor at http://localhost:<port> and opens it in your browser.

Options:
  -p, --port <PORT>   Port to listen on (default 8080, or $DOCX_EDITOR_PORT; 0 = any free port)
      --no-open       Do not open the browser automatically
  -h, --help          Show this help
  -V, --version       Show the version

The server only listens on 127.0.0.1 and never contacts the internet.";

/// Parses command-line arguments (without the program name). `env_port` is
/// the value of `DOCX_EDITOR_PORT`, if set; `--port` takes precedence.
pub fn parse_args<I>(args: I, env_port: Option<&str>) -> Result<Command, ConfigError>
where
    I: IntoIterator<Item = String>,
{
    let mut config = Config::default();
    if let Some(raw) = env_port.map(str::trim).filter(|s| !s.is_empty()) {
        config.port = parse_port(raw).map_err(|e| ConfigError(format!("{PORT_ENV}: {e}")))?;
    }

    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        let (flag, inline_value) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag.to_string(), Some(value.to_string())),
            _ => (arg.clone(), None),
        };
        match flag.as_str() {
            "-h" | "--help" => return Ok(Command::Help),
            "-V" | "--version" => return Ok(Command::Version),
            "--no-open" => config.open_browser = false,
            "-p" | "--port" => {
                let value = inline_value
                    .or_else(|| args.next())
                    .ok_or_else(|| ConfigError("--port requires a value".into()))?;
                config.port = parse_port(&value).map_err(|e| ConfigError(format!("--port: {e}")))?;
            }
            _ => return Err(ConfigError(format!("unknown option '{arg}' (see --help)"))),
        }
    }
    Ok(Command::Run(config))
}

fn parse_port(raw: &str) -> Result<u16, String> {
    raw.trim().parse::<u16>().map_err(|_| format!("'{raw}' is not a valid port (0-65535)"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn defaults() {
        let cmd = parse_args(args(&[]), None).unwrap();
        assert_eq!(cmd, Command::Run(Config { port: 8080, open_browser: true }));
    }

    #[test]
    fn binds_to_loopback_only() {
        let addr = Config::default().bind_addr();
        assert!(addr.ip().is_loopback());
        assert_eq!(addr.to_string(), "127.0.0.1:8080");
    }

    #[test]
    fn port_and_no_open() {
        for form in [&["--port", "9000", "--no-open"][..], &["--port=9000", "--no-open"], &["-p", "9000", "--no-open"]]
        {
            let cmd = parse_args(args(form), None).unwrap();
            assert_eq!(cmd, Command::Run(Config { port: 9000, open_browser: false }), "{form:?}");
        }
    }

    #[test]
    fn env_port_is_used_and_flag_wins() {
        assert_eq!(
            parse_args(args(&[]), Some("7000")).unwrap(),
            Command::Run(Config { port: 7000, open_browser: true })
        );
        assert_eq!(
            parse_args(args(&["--port", "7001"]), Some("7000")).unwrap(),
            Command::Run(Config { port: 7001, open_browser: true })
        );
        assert_eq!(parse_args(args(&[]), Some("  ")).unwrap(), Command::Run(Config::default()));
    }

    #[test]
    fn help_and_version() {
        assert_eq!(parse_args(args(&["--help"]), None).unwrap(), Command::Help);
        assert_eq!(parse_args(args(&["-h"]), None).unwrap(), Command::Help);
        assert_eq!(parse_args(args(&["-V"]), None).unwrap(), Command::Version);
    }

    #[test]
    fn invalid_input() {
        for (form, env, needle) in [
            (&["--port"][..], None, "requires a value"),
            (&["--port", "eighty"], None, "not a valid port"),
            (&["--port", "70000"], None, "not a valid port"),
            (&["--port", "-1"], None, "not a valid port"),
            (&["--verbose"], None, "unknown option"),
            (&[], Some("abc"), "DOCX_EDITOR_PORT"),
        ] {
            let err = parse_args(args(form), env).unwrap_err();
            assert!(err.0.contains(needle), "{form:?}/{env:?}: {err}");
        }
    }
}
