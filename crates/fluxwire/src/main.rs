//! The `fluxwire` binary. With no subcommand it opens the GUI; built with
//! `--no-default-features` it contains no GPUI code at all.

fn main() {
    #[cfg(feature = "gui")]
    {
        fluxwire_ui::run();
    }
    #[cfg(not(feature = "gui"))]
    {
        eprintln!("fluxwire: built without the `gui` feature");
        std::process::exit(64);
    }
}
