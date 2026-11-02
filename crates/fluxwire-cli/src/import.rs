//! `fluxwire import <source> <SRC> --into <DIR> [--report FILE]`.

use crate::exit;
use clap::Args;
use fluxwire_import::{ImportError, Options, Source, apply, detect, plan};
use std::path::PathBuf;
use std::process::ExitCode;

/// Reading the source or writing the target failed (BSD `EX_IOERR`).
const IO_ERROR: u8 = 74;

#[derive(Args, Debug)]
pub struct ImportArgs {
    /// postman, insomnia, openapi, hurl or http (use `auto` to detect from the file).
    #[arg(value_name = "SOURCE")]
    pub source: String,
    /// The file to import (a directory of `.hurl` files for hurl).
    #[arg(value_name = "SRC")]
    pub src: PathBuf,
    /// Folder to write into. Existing files are never overwritten unless --overwrite is given.
    #[arg(long, value_name = "DIR")]
    pub into: PathBuf,
    /// Write the import report (Markdown) to this file.
    #[arg(long, value_name = "FILE")]
    pub report: Option<PathBuf>,
    /// Overwrite files that already exist in --into.
    #[arg(long)]
    pub overwrite: bool,
    /// Plan only: print the report and write nothing.
    #[arg(long)]
    pub dry_run: bool,
    /// OpenAPI: copy the spec into the collection and create a Spec link.
    #[arg(long)]
    pub spec_link: bool,
    /// Collection name (default: the name in the source).
    #[arg(long, value_name = "NAME")]
    pub name: Option<String>,
}

pub fn run(a: ImportArgs) -> ExitCode {
    let source = if a.source == "auto" {
        let text = std::fs::read_to_string(&a.src).unwrap_or_default();
        match detect(&a.src, &text) {
            Some(s) => s,
            None => {
                eprintln!(
                    "fluxwire: could not detect the source of {}",
                    a.src.display()
                );
                return ExitCode::from(exit::USAGE);
            }
        }
    } else {
        match Source::from_id(&a.source) {
            Some(s) => s,
            None => {
                eprintln!(
                    "fluxwire: unknown source `{}` (postman, insomnia, openapi, hurl, http, auto)",
                    a.source
                );
                return ExitCode::from(exit::USAGE);
            }
        }
    };
    let opts = Options {
        spec_link: a.spec_link,
        name: a.name.clone(),
    };
    let mut p = match plan(source, &a.src, &opts) {
        Ok(p) => p,
        Err(e @ ImportError::Parse(_)) => {
            eprintln!("fluxwire: {e}");
            return ExitCode::from(exit::PARSE);
        }
        Err(e) => {
            eprintln!("fluxwire: {e}");
            return ExitCode::from(IO_ERROR);
        }
    };
    let written = if a.dry_run {
        Vec::new()
    } else {
        let overwrite = a.overwrite;
        match apply(&mut p, &a.into, &|_| overwrite) {
            Ok(w) => w,
            Err(e) => {
                eprintln!("fluxwire: {}: {e}", a.into.display());
                return ExitCode::from(IO_ERROR);
            }
        }
    };
    let md = p.report.to_markdown(&p.files);
    if let Some(r) = &a.report
        && let Err(e) = std::fs::write(r, &md)
    {
        eprintln!("fluxwire: {}: {e}", r.display());
        return ExitCode::from(IO_ERROR);
    }
    if a.dry_run {
        println!(
            "Would import {} files into {}",
            p.files.len(),
            a.into.display()
        );
    } else {
        println!("Imported {} files into {}", written.len(), a.into.display());
    }
    for s in &p.report.skipped {
        println!("skipped (exists): {}", s.display());
    }
    for l in &p.report.lossy {
        let at = if l.file.is_empty() {
            String::new()
        } else {
            format!("{}: ", l.file)
        };
        println!("lossy: {at}{}. {}", l.feature, l.reason);
    }
    ExitCode::SUCCESS
}
