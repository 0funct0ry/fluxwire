//! `fmt`, `validate`, `env list`, `completions`.

use crate::{exit, files};
use clap::{Args, CommandFactory};
use clap_complete::Shell;
use fluxwire_format::{file_kind_for, format, is_canonical};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct FmtArgs {
    /// Report non-canonical files and exit 1 instead of rewriting.
    #[arg(long)]
    pub check: bool,
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,
}

pub fn fmt(a: FmtArgs) -> ExitCode {
    let mut bad = 0usize;
    let mut code = 0;
    for p in &a.paths {
        if !p.exists() {
            eprintln!("fluxwire: {}: no such file or directory", p.display());
            return ExitCode::from(exit::USAGE);
        }
        for f in files::collect(p) {
            let Ok(src) = std::fs::read_to_string(&f) else {
                eprintln!("{}: cannot read", f.display());
                code = exit::merge(code, exit::PARSE);
                continue;
            };
            let kind = file_kind_for(&f);
            if is_canonical(&src, kind) {
                continue;
            }
            bad += 1;
            if a.check {
                println!("{}", f.display());
            } else if let Err(e) = std::fs::write(&f, format(&src, kind)) {
                eprintln!("{}: {e}", f.display());
                code = exit::merge(code, 1);
            } else {
                println!("formatted {}", f.display());
            }
        }
    }
    if a.check && bad > 0 {
        eprintln!("{bad} file(s) are not canonically formatted; run `fluxwire fmt`");
        code = exit::merge(code, exit::TESTS_FAILED);
    }
    ExitCode::from(code)
}

#[derive(Args, Debug)]
pub struct ValidateArgs {
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,
}

pub fn validate(a: ValidateArgs) -> ExitCode {
    let mut code = 0;
    for p in &a.paths {
        if !p.exists() {
            eprintln!("fluxwire: {}: no such file or directory", p.display());
            return ExitCode::from(exit::USAGE);
        }
        code = exit::merge(code, files::check(&files::collect(p), true));
    }
    ExitCode::from(code)
}

#[derive(Args, Debug)]
pub struct EnvListArgs {
    pub path: PathBuf,
}

pub fn env_list(a: EnvListArgs) -> ExitCode {
    let Ok(abs) = a.path.canonicalize() else {
        eprintln!("fluxwire: {}: no such directory", a.path.display());
        return ExitCode::from(exit::USAGE);
    };
    let root = fluxwire_runner::prepare::collection_root_of(&abs).unwrap_or(abs);
    for name in fluxwire_format::list_environments(&root) {
        println!("{name}");
    }
    ExitCode::SUCCESS
}

#[derive(Args, Debug)]
pub struct CompletionsArgs {
    pub shell: Shell,
}

pub fn completions(a: CompletionsArgs) -> ExitCode {
    let mut cmd = crate::Cli::command();
    clap_complete::generate(a.shell, &mut cmd, "fluxwire", &mut std::io::stdout());
    ExitCode::SUCCESS
}
