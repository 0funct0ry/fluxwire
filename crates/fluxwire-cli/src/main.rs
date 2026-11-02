//! `fluxwire-cli`: the headless CLI, built without GPUI.

fn main() -> std::process::ExitCode {
    fluxwire_cli::run(std::env::args_os())
}
