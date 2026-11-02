//! clap command tree and reporters (no GPUI). `mcp serve` (ADR-009) and the M10 headless CLI.

mod data;
mod exit;
mod files;
mod import;
mod mcp;
mod reporter;
mod run;
mod tools;

use clap::{Parser, Subcommand};
use std::ffi::OsString;
use std::process::ExitCode;

#[derive(Parser)]
#[command(
    name = "fluxwire",
    version,
    about = "Fluxwire API client",
    override_usage = "fluxwire [WORKSPACE_PATH]\n       fluxwire <COMMAND>",
    after_help = "With no command, `fluxwire [WORKSPACE_PATH]` opens the workspace in the app\n(the current directory's session when no path is given)."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Subcommand)]
pub enum Command {
    /// Run requests, folders or collections headlessly.
    Run(run::RunArgs),
    /// Send one request and print the response body (status and timing go to stderr).
    Send(run::SendArgs),
    /// Canonical formatting.
    Fmt(tools::FmtArgs),
    /// Parse files and report diagnostics.
    Validate(tools::ValidateArgs),
    /// Environment commands.
    Env {
        #[command(subcommand)]
        command: EnvCommand,
    },
    /// Print a shell completion script.
    Completions(tools::CompletionsArgs),
    /// Import a Postman, Insomnia, OpenAPI, Hurl or .http source into a folder (SPEC §14).
    Import(import::ImportArgs),
    /// OpenAPI sync (M12).
    #[command(hide = true)]
    Openapi {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// API documentation export (M13).
    #[command(hide = true)]
    Docs {
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },
    /// Model Context Protocol server for agents (ADR-009).
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
}

#[derive(Subcommand)]
pub enum EnvCommand {
    /// List the environments of a Workspace.
    List(tools::EnvListArgs),
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
    matches!(
        arg,
        "run"
            | "send"
            | "fmt"
            | "validate"
            | "env"
            | "completions"
            | "import"
            | "openapi"
            | "docs"
            | "mcp"
            | "--version"
            | "-V"
            | "--help"
            | "-h"
            | "help"
    )
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
                ExitCode::from(exit::USAGE)
            } else {
                ExitCode::SUCCESS
            };
        }
    };
    match cli.command {
        Command::Run(a) => run::run(a),
        Command::Send(a) => run::send(a),
        Command::Fmt(a) => tools::fmt(a),
        Command::Validate(a) => tools::validate(a),
        Command::Env {
            command: EnvCommand::List(a),
        } => tools::env_list(a),
        Command::Completions(a) => tools::completions(a),
        Command::Import(a) => import::run(a),
        Command::Openapi { .. } => tools::stub("openapi", "M12"),
        Command::Docs { .. } => tools::stub("docs export", "M13"),
        Command::Mcp {
            command: McpCommand::Serve(a),
        } => mcp::serve(a),
        Command::Mcp {
            command: McpCommand::Scenario(a),
        } => mcp::scenario(a),
    }
}
