//! `fluxwire mcp serve`.

use clap::Args;
use fluxwire_agent::ServeConfig;
use fluxwire_format::McpSettings;
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Args, Debug)]
pub struct ServeArgs {
    /// stdio, unix or both (default stdio; `unix` and `both` are Unix-only).
    #[arg(long, value_name = "stdio|unix|both")]
    pub transport: Option<String>,
    /// Socket path for the unix transport (default: per-user runtime dir, mcp-<pid>.sock).
    #[arg(long, value_name = "PATH")]
    pub socket: Option<PathBuf>,
    /// Tool classes to expose (default read,execute).
    #[arg(long, value_name = "CLASSES", value_delimiter = ',')]
    pub allow: Option<Vec<String>>,
    /// Workspace allow-list; repeatable.
    #[arg(long, value_name = "PATH")]
    pub root: Vec<PathBuf>,
}

/// `[mcp]` of the first root's fluxwire.toml (or the current directory's), then user settings.
fn file_layers(roots: &[PathBuf]) -> (McpSettings, McpSettings) {
    let base = roots.first().cloned().unwrap_or_else(|| PathBuf::from("."));
    let manifest = std::fs::read_to_string(base.join("fluxwire.toml"))
        .map(|t| fluxwire_format::parse_manifest_mcp(&t))
        .unwrap_or_default();
    let user = directories::ProjectDirs::from("dev", "fluxwire", "fluxwire")
        .map(|d| fluxwire_format::load_settings(d.config_dir()).mcp)
        .unwrap_or_default();
    (manifest, user)
}

pub fn serve(a: ServeArgs) -> ExitCode {
    let (manifest, user) = file_layers(&a.root);
    let flags = McpSettings {
        transport: a.transport,
        socket: a.socket,
        allow: a.allow,
        roots: (!a.root.is_empty()).then_some(a.root),
    };
    let cfg = match ServeConfig::resolve(flags, manifest, user) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("fluxwire: {e}");
            return ExitCode::from(64);
        }
    };
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("fluxwire: cannot start the runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match rt.block_on(fluxwire_agent::serve(cfg)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("fluxwire: {e:#}");
            ExitCode::FAILURE
        }
    }
}

#[derive(Args, Debug)]
pub struct ScenarioArgs {
    /// Scenario file, such as fixtures/scenarios/m03-send.yaml.
    pub file: PathBuf,
    /// Socket of the app started with `--mcp-socket`.
    #[arg(long, value_name = "PATH")]
    pub socket: PathBuf,
    /// Skip screenshot checkpoints (no display or Screen Recording permission).
    #[arg(long)]
    pub no_capture: bool,
    /// Write checkpoint and failure screenshots here.
    #[arg(long, value_name = "DIR")]
    pub out: Option<PathBuf>,
}

pub fn scenario(a: ScenarioArgs) -> ExitCode {
    let rt = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("fluxwire: cannot start the runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    let out = rt.block_on(fluxwire_agent::scenario_client::run_file(
        &a.file,
        &a.socket,
        !a.no_capture,
    ));
    match out {
        Err(e) => {
            eprintln!("fluxwire: {e}");
            ExitCode::from(2)
        }
        Ok(o) => {
            print!("{}", o.summary());
            if let Some(dir) = &a.out {
                let _ = std::fs::create_dir_all(dir);
                for (i, (label, png)) in o.images.iter().enumerate() {
                    let name: String = label
                        .chars()
                        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
                        .collect();
                    let _ = std::fs::write(dir.join(format!("{i:02}-{name}.png")), png);
                }
            }
            if o.passed() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(1)
            }
        }
    }
}
