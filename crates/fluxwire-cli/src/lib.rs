//! clap command tree and reporters (no GPUI). M9.1 has only `mcp serve` and `--version`;
//! M10 extends this tree.

mod mcp;

use clap::{Parser, Subcommand};
use std::ffi::OsString;
use std::process::ExitCode;

#[derive(Parser)]
#[command(name = "fluxwire", version, about = "Fluxwire API client")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Model Context Protocol server for agents (ADR-009).
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
}

#[derive(Subcommand)]
pub enum McpCommand {
    /// Serve Fluxwire tools over stdio and/or a unix socket. Logs go to stderr.
    Serve(mcp::ServeArgs),
    /// Run a scenario file against a running `dev-mcp` app over its socket. Exit 0 when every
    /// step passes, 1 when one fails, 2 when the app cannot be reached.
    Scenario(mcp::ScenarioArgs),
}

/// True when `arg` (the first argument after the program name) belongs to the CLI tree.
pub fn is_cli_arg(arg: &str) -> bool {
    matches!(arg, "mcp" | "--version" | "-V" | "--help" | "-h" | "help")
}

/// Runs the CLI and returns the process exit code (64 for usage errors).
pub fn run<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    let cli = match Cli::try_parse_from(args) {
        Ok(c) => c,
        Err(e) => {
            let usage = e.use_stderr();
            let _ = e.print();
            return if usage {
                ExitCode::from(64)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    match cli.command {
        Command::Mcp {
            command: McpCommand::Serve(a),
        } => mcp::serve(a),
        Command::Mcp {
            command: McpCommand::Scenario(a),
        } => mcp::scenario(a),
    }
}
