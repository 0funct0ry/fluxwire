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
    #[cfg(feature = "gui")]
    fluxwire_ui::run();
    ExitCode::SUCCESS
}
