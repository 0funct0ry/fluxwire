//! The `fluxwire` binary. With no subcommand it opens the GUI; built with
//! `--no-default-features` it contains no GPUI code at all.

use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<std::ffi::OsString> = std::env::args_os().collect();
    let is_cli = args
        .get(1)
        .and_then(|a| a.to_str())
        .is_some_and(fluxwire_cli::is_cli_arg);
    if is_cli || cfg!(not(feature = "gui")) {
        if args.len() < 2 {
            eprintln!("fluxwire: built without the `gui` feature; try `fluxwire mcp serve`");
            return ExitCode::from(64);
        }
        return fluxwire_cli::run(args);
    }
    #[cfg(all(feature = "gui", not(feature = "dev-mcp")))]
    fluxwire_ui::run();
    #[cfg(feature = "dev-mcp")]
    run_dev(&args);
    ExitCode::SUCCESS
}

/// `fluxwire [--mcp-socket [PATH]] [--mcp-stdio] [WORKSPACE]` in `dev-mcp` builds.
#[cfg(feature = "dev-mcp")]
fn run_dev(args: &[std::ffi::OsString]) {
    let mut host = fluxwire_ui::HostOpts::default();
    let mut path = None;
    let mut it = args.iter().skip(1).peekable();
    while let Some(a) = it.next() {
        match a.to_str() {
            Some("--mcp-stdio") => host.stdio = true,
            Some("--mcp-socket") => {
                let given = it
                    .next_if(|n| {
                        !n.to_string_lossy().starts_with("--") && !std::path::Path::new(n).is_dir()
                    })
                    .map(std::path::PathBuf::from);
                host.socket = Some(given);
            }
            _ => path = Some(std::path::PathBuf::from(a)).filter(|p| p.is_dir()),
        }
    }
    fluxwire_ui::run_with(path, host);
}
