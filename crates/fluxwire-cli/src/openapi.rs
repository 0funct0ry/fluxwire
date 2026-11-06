//! `fluxwire openapi status | pull | push | diff [--link NAME] [--yes]` (SPEC §15, §18).

use crate::exit;
use clap::{Args, Subcommand};
use fluxwire_model::OpenApiLink;
use fluxwire_openapi::link::{self, LinkState};
use fluxwire_openapi::push::Deletion;
use std::io::{IsTerminal, Write};
use std::path::PathBuf;
use std::process::ExitCode;

const IO_ERROR: u8 = 74;

#[derive(Args, Debug)]
pub struct OpenapiArgs {
    #[command(subcommand)]
    pub command: OpenapiCommand,
    /// Collection root (default: the current directory).
    #[arg(long, global = true, value_name = "DIR", default_value = ".")]
    pub root: PathBuf,
    /// Spec link name (default: every link).
    #[arg(long, global = true, value_name = "NAME")]
    pub link: Option<String>,
    /// Do not ask for confirmation.
    #[arg(long, short = 'y', global = true)]
    pub yes: bool,
}

#[derive(Subcommand, Debug, Clone, Copy)]
pub enum OpenapiCommand {
    /// Show incoming, outgoing and conflicting changes.
    Status,
    /// Apply incoming spec changes to the collection.
    Pull,
    /// Apply outgoing collection changes to the spec.
    Push {
        /// Remove deleted operations instead of marking them deprecated.
        #[arg(long)]
        remove: bool,
    },
    /// Print every change, and the patch a push would make.
    Diff,
}

fn confirm(prompt: &str, yes: bool) -> bool {
    if yes {
        return true;
    }
    if !std::io::stdin().is_terminal() {
        eprintln!("fluxwire: {prompt} — pass --yes to proceed without a terminal");
        return false;
    }
    eprint!("{prompt} [y/N] ");
    let _ = std::io::stderr().flush();
    let mut s = String::new();
    std::io::stdin().read_line(&mut s).is_ok() && matches!(s.trim(), "y" | "Y" | "yes")
}

fn describe(c: &fluxwire_openapi::Change) -> String {
    use fluxwire_openapi::{ChangeKind, Direction};
    let arrow = if c.direction == Direction::Incoming {
        "↓"
    } else {
        "↑"
    };
    match &c.kind {
        ChangeKind::Field { field, value } => format!("{arrow} {} {field} = {value}", c.operation),
        ChangeKind::Add(_) => format!("{arrow} {} added", c.operation),
        ChangeKind::Delete => format!("{arrow} {} deleted", c.operation),
    }
}

pub fn run(a: OpenapiArgs) -> ExitCode {
    let cfg = match fluxwire_format::load_config(&a.root) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("fluxwire: {e}");
            return ExitCode::from(exit::PARSE);
        }
    };
    let links: Vec<&OpenApiLink> = cfg
        .openapi
        .iter()
        .filter(|l| a.link.as_deref().is_none_or(|n| n == l.name))
        .collect();
    if links.is_empty() {
        eprintln!("fluxwire: no matching [[openapi]] link in fluxwire.toml");
        return ExitCode::from(exit::USAGE);
    }
    let mut code = 0u8;
    for l in links {
        let state = match link::load(&a.root, l, None) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("fluxwire: {}: {e}", l.name);
                code = exit::merge(
                    code,
                    if matches!(e, link::SyncError::Io(_)) {
                        IO_ERROR
                    } else {
                        exit::PARSE
                    },
                );
                continue;
            }
        };
        code = exit::merge(code, one(&a, &state));
    }
    ExitCode::from(code)
}

fn one(a: &OpenapiArgs, st: &LinkState) -> u8 {
    let s = st.status();
    let name = &st.link.name;
    let summary = format!(
        "{name}: {} incoming, {} outgoing, {} conflicts{}",
        s.incoming,
        s.outgoing,
        s.conflicts,
        if s.needs_manual > 0 {
            format!(", {} need manual edit", s.needs_manual)
        } else {
            String::new()
        }
    );
    match a.command {
        OpenapiCommand::Status => {
            println!("{summary}");
            if s.conflicts > 0 { exit::CONFLICTS } else { 0 }
        }
        OpenapiCommand::Diff => {
            println!("{summary}");
            for c in &st.result.changes {
                println!("  {}", describe(c));
            }
            for c in st.conflicts() {
                println!(
                    "  ! {} {} conflicts",
                    c.operation,
                    c.field.as_deref().unwrap_or("(operation)")
                );
            }
            if let Ok((p, _)) = link::push_preview(st, &st.result.changes, Deletion::Deprecate)
                && p.text != st.spec_text
            {
                println!("\nspec after push:\n{}", p.text);
            }
            if s.conflicts > 0 { exit::CONFLICTS } else { 0 }
        }
        OpenapiCommand::Pull | OpenapiCommand::Push { .. } => {
            if s.conflicts > 0 {
                eprintln!(
                    "fluxwire: {name}: {} conflict(s) must be resolved first (use the Sync tab)",
                    s.conflicts
                );
                return exit::CONFLICTS;
            }
            let pulling = matches!(a.command, OpenapiCommand::Pull);
            let n = if pulling { s.incoming } else { s.outgoing };
            if n == 0 {
                println!(
                    "{name}: nothing to {}",
                    if pulling { "pull" } else { "push" }
                );
                return 0;
            }
            for c in st
                .result
                .changes
                .iter()
                .filter(|c| (c.direction == fluxwire_openapi::Direction::Incoming) == pulling)
            {
                println!("  {}", describe(c));
            }
            if !confirm(&format!("{name}: apply {n} change(s)?"), a.yes) {
                return exit::USAGE;
            }
            if pulling {
                match link::pull(st, &st.result.changes) {
                    Ok(r) => {
                        println!(
                            "{name}: updated {} file(s), created {}, removed {}",
                            r.files_changed.len(),
                            r.files_created.len(),
                            r.files_trashed.len()
                        );
                        0
                    }
                    Err(e) => {
                        eprintln!("fluxwire: {name}: {e}");
                        IO_ERROR
                    }
                }
            } else {
                let del = if matches!(a.command, OpenapiCommand::Push { remove: true }) {
                    Deletion::Remove
                } else {
                    Deletion::Deprecate
                };
                match link::push(st, &st.result.changes, del) {
                    Ok(r) => {
                        println!(
                            "{name}: spec {}",
                            if r.written { "updated" } else { "unchanged" }
                        );
                        for m in &r.manual {
                            println!(
                                "  needs manual edit: {} {} — {}{}",
                                m.operation,
                                m.field,
                                m.reason,
                                m.pointer
                                    .as_deref()
                                    .map(|p| format!(" ({p})"))
                                    .unwrap_or_default()
                            );
                        }
                        for (e, why) in &r.patch {
                            println!("  patch only: {:?} = {:?} — {why}", e.path, e.value);
                        }
                        0
                    }
                    Err(e) => {
                        eprintln!("fluxwire: {name}: {e}");
                        if matches!(e, link::SyncError::Remote) {
                            exit::USAGE
                        } else {
                            IO_ERROR
                        }
                    }
                }
            }
        }
    }
}
