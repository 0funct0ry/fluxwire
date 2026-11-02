//! `fluxwire run` and `fluxwire send`.

use crate::data::{self, Row};
use crate::reporter::{self, Reporter, Style};
use crate::{exit, files};
use clap::Args;
use fluxwire_runner::prepare::{FolderOpts, Prep, collection_root_of};
use fluxwire_runner::{FolderRunEvent, RunEvent, runner};
use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct CommonArgs {
    /// Environment to use.
    #[arg(short, long, value_name = "NAME")]
    pub env: Option<String>,
    /// Variable override; repeatable.
    #[arg(long = "var", value_name = "NAME=VALUE")]
    pub vars: Vec<String>,
    /// Skip TLS verification (prints a warning).
    #[arg(long)]
    pub insecure: bool,
}

#[derive(Args, Debug)]
pub struct RunArgs {
    /// Requests, folders or collections.
    #[arg(required = true)]
    pub paths: Vec<PathBuf>,
    #[command(flatten)]
    pub common: CommonArgs,
    /// Extra dotenv file (just below --var).
    #[arg(long, value_name = "FILE")]
    pub env_file: Option<PathBuf>,
    /// Max parallel requests.
    #[arg(long, value_name = "N")]
    pub concurrency: Option<u32>,
    /// Force sequential mode.
    #[arg(long)]
    pub sequential: bool,
    /// CSV or JSON data file for iterations.
    #[arg(long, value_name = "FILE")]
    pub data: Option<PathBuf>,
    #[arg(long, value_name = "N")]
    pub iterations: Option<u32>,
    /// Filter, e.g. tag:smoke or name:*refund*.
    #[arg(long, value_name = "EXPR")]
    pub filter: Option<String>,
    /// Stop at first failure.
    #[arg(long)]
    pub bail: bool,
    /// Per-request timeout override in milliseconds.
    #[arg(long, value_name = "MS")]
    pub timeout: Option<u64>,
    /// Reporter; repeatable.
    #[arg(short, long, value_enum)]
    pub reporter: Vec<Reporter>,
    /// Output file for the last reporter.
    #[arg(short, long, value_name = "FILE")]
    pub output: Option<PathBuf>,
    #[arg(long)]
    pub no_color: bool,
    #[arg(long)]
    pub quiet: bool,
    #[arg(long)]
    pub verbose: bool,
}

fn usage(msg: impl std::fmt::Display) -> u8 {
    eprintln!("fluxwire: {msg}");
    exit::USAGE
}

fn config_dir() -> Option<PathBuf> {
    directories::ProjectDirs::from("dev", "fluxwire", "fluxwire")
        .map(|d| d.config_dir().to_path_buf())
}

fn prep(c: &CommonArgs, env_file: Option<&Path>) -> Result<Prep, String> {
    let mut p = Prep::new(config_dir(), c.env.clone(), None);
    // The CLI always allows process.env (SPEC §8).
    p.settings.allow_process_env = true;
    if let Some(f) = env_file {
        let text = std::fs::read_to_string(f).map_err(|e| format!("{}: {e}", f.display()))?;
        for (k, v) in data::parse_dotenv(&text) {
            p.runtime.insert(k, (v, false));
        }
    }
    for v in &c.vars {
        let (k, v) = data::parse_var(v)?;
        p.runtime.insert(k, (v, false));
    }
    if c.insecure {
        eprintln!("warning: TLS verification is disabled (--insecure)");
    }
    Ok(p)
}

pub fn run(a: RunArgs) -> ExitCode {
    ExitCode::from(run_inner(a))
}

fn run_inner(a: RunArgs) -> u8 {
    let rows: Vec<Row> = match a.data.as_deref().map(data::load_data).transpose() {
        Ok(r) => r.unwrap_or_default(),
        Err(e) => return usage(e),
    };
    let p = match prep(&a.common, a.env_file.as_deref()) {
        Ok(p) => p,
        Err(e) => return usage(e),
    };
    let reporters = if a.reporter.is_empty() {
        vec![Reporter::Pretty]
    } else {
        a.reporter.clone()
    };
    let style = Style {
        color: !a.no_color
            && std::env::var_os("NO_COLOR").is_none()
            && std::io::stdout().is_terminal(),
        verbose: a.verbose,
    };
    // Validate everything first so a bad file is 65/66 before any request is sent.
    let mut code = 0;
    let mut targets = Vec::new();
    for path in &a.paths {
        let Ok(abs) = path.canonicalize() else {
            return usage(format!("{}: no such file or directory", path.display()));
        };
        let Some(root) = collection_root_of(&abs) else {
            return usage(format!(
                "{}: not inside a collection (no fluxwire.toml)",
                path.display()
            ));
        };
        code = exit::merge(code, files::check(&files::collect(&abs), false));
        targets.push((abs, root));
    }
    if code != 0 {
        return code;
    }
    let mut buffers = vec![String::new(); reporters.len()];
    for (abs, root) in targets {
        let (folder, only) = if abs.is_file() {
            let parent = abs.parent().unwrap_or(&root);
            (
                fluxwire_format::rel_key(&root, parent),
                Some(vec![fluxwire_format::rel_key(&root, &abs)]),
            )
        } else {
            (fluxwire_format::rel_key(&root, &abs), None)
        };
        let opts = FolderOpts {
            sequential: a.sequential,
            concurrency: a.concurrency,
            iterations: a.iterations,
            data: rows.clone(),
            filter: a.filter.clone(),
            only,
            bail: a.bail,
            delay_ms: 0,
            timeout_ms: a.timeout,
        };
        let mut plan = p.run_plan(&root, &folder, opts);
        plan.scripts.trusted = true; // `fluxwire run` trusts by definition (SPEC §21)
        if a.common.insecure {
            plan.settings.verify_tls = false;
        }
        let mut handle = runner().run_folder(plan);
        let result = runner().handle().block_on(async {
            while let Some(ev) = handle.events.recv().await {
                if let FolderRunEvent::Finished(r) = ev {
                    return Some(r);
                }
            }
            None
        });
        let Some(result) = result else {
            eprintln!("fluxwire: the run ended without a result");
            return 1;
        };
        code = exit::merge(code, exit::run_code(&result));
        for (i, r) in reporters.iter().enumerate() {
            buffers[i].push_str(&reporter::render(*r, &result, style));
        }
    }
    for (i, (r, text)) in reporters.iter().zip(&buffers).enumerate() {
        let last = i + 1 == reporters.len();
        if last && let Some(out) = &a.output {
            if let Err(e) = std::fs::write(out, text) {
                eprintln!("fluxwire: {}: {e}", out.display());
                return 1;
            }
        } else if !(a.quiet && *r == Reporter::Pretty) {
            print!("{text}");
        }
    }
    code
}

#[derive(Args, Debug)]
pub struct SendArgs {
    pub file: PathBuf,
    #[command(flatten)]
    pub common: CommonArgs,
    /// Print the status line and headers before the body.
    #[arg(short = 'i', long)]
    pub include: bool,
}

pub fn send(a: SendArgs) -> ExitCode {
    ExitCode::from(send_inner(a))
}

fn send_inner(a: SendArgs) -> u8 {
    let Ok(abs) = a.file.canonicalize() else {
        return usage(format!("{}: no such file", a.file.display()));
    };
    let c = files::check(std::slice::from_ref(&abs), false);
    if c != 0 {
        return c;
    }
    let p = match prep(&a.common, None) {
        Ok(p) => p,
        Err(e) => return usage(e),
    };
    let src = std::fs::read_to_string(&abs).unwrap_or_default();
    let doc = fluxwire_format::parse_with(
        &src,
        fluxwire_format::file_kind_for(&abs),
        &fluxwire_protocol::ProtocolRegistry::builtin(),
    );
    let root = collection_root_of(&abs)
        .unwrap_or_else(|| abs.parent().map(Path::to_path_buf).unwrap_or_default());
    let mut opts = p.send_options(&doc, &abs, &root);
    opts.scripts.trusted = true;
    if a.common.insecure {
        opts.settings.verify_tls = false;
    }
    let mut handle = runner().send(&doc, opts);
    let res = runner().handle().block_on(async {
        while let Some(ev) = handle.events.recv().await {
            if let RunEvent::Finished(r) = ev {
                return Some(r);
            }
        }
        None
    });
    let Some(res) = res else {
        eprintln!("fluxwire: the send ended without a result");
        return 1;
    };
    if let Some(e) = &res.error {
        let (msg, fix) = e.describe();
        eprintln!("{}", res.redact(&msg));
        if let Some(f) = fix {
            eprintln!("{f}");
        }
    }
    if let Some(ex) = res.outcome.as_ref().and_then(|o| o.exchange.as_ref()) {
        let ms = res.outcome.as_ref().map(|o| o.total_ms).unwrap_or(0.0);
        eprintln!(
            "{} {} {}  {:.0}ms",
            ex.version, ex.status, ex.status_text, ms
        );
        let mut out = std::io::stdout().lock();
        if a.include {
            let _ = writeln!(out, "{} {} {}", ex.version, ex.status, ex.status_text);
            for (k, v) in &ex.headers {
                let _ = writeln!(out, "{k}: {v}");
            }
            let _ = writeln!(out);
        }
        if let Ok(b) = ex.body.read_all() {
            let _ = out.write_all(res.redact(&String::from_utf8_lossy(&b)).as_bytes());
        }
    }
    for t in res.tests.iter().filter(|t| !t.passed) {
        eprintln!("FAIL {}", res.redact(&t.name));
    }
    exit::result_code(&res)
}
