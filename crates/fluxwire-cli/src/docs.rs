//! `fluxwire docs export <PATH> --out <DIR>` (SPEC §16): a static, single-page HTML reference.

use crate::exit;
use clap::{Args, Subcommand};
use fluxwire_docs::export::{ExportError, export};
use std::path::PathBuf;
use std::process::ExitCode;

const IO_ERROR: u8 = 74;

#[derive(Args, Debug)]
pub struct DocsArgs {
    #[command(subcommand)]
    pub command: DocsCommand,
}

#[derive(Subcommand, Debug)]
pub enum DocsCommand {
    /// Write a single-page, no-JavaScript HTML reference of a collection, folder or file.
    Export {
        /// A collection or folder directory, or one `.flux.md` / `README.md` file.
        #[arg(value_name = "PATH")]
        path: PathBuf,
        /// Directory to write `index.html` (and `assets/`) into.
        #[arg(long, value_name = "DIR")]
        out: PathBuf,
    },
}

pub fn run(a: DocsArgs) -> ExitCode {
    let DocsCommand::Export { path, out } = a.command;
    match export(&path, &out) {
        Ok(e) => {
            println!(
                "wrote {} ({} file{}, {} image{})",
                e.index.display(),
                e.files,
                if e.files == 1 { "" } else { "s" },
                e.images,
                if e.images == 1 { "" } else { "s" }
            );
            ExitCode::SUCCESS
        }
        Err(ExportError::NotFound(m)) => {
            eprintln!("fluxwire docs export: {m}");
            ExitCode::from(exit::USAGE)
        }
        Err(ExportError::Io(e)) => {
            eprintln!("fluxwire docs export: {e}");
            ExitCode::from(IO_ERROR)
        }
    }
}
