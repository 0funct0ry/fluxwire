//! `cli: install shell command`: a `fluxwire` symlink on the user's PATH that points at the app
//! binary (macOS/Linux). The planning and file operations are plain functions so they are tested
//! without GPUI; the actions at the bottom only report the result as a notice.

use std::path::{Path, PathBuf};

use gpui::Context;

use crate::workspace::WorkspaceView;

pub const SHIM_NAME: &str = "fluxwire";

/// What an install did.
#[derive(Debug, PartialEq, Eq)]
pub enum Installed {
    Created(PathBuf),
    AlreadyPresent(PathBuf),
}

/// Where the shim goes: `~/.local/bin` under `home`.
pub fn shim_dir(home: &Path) -> PathBuf {
    home.join(".local").join("bin")
}

pub fn on_path(dir: &Path, path_var: Option<&std::ffi::OsStr>) -> bool {
    path_var.is_some_and(|p| std::env::split_paths(p).any(|d| d == dir))
}

#[cfg(unix)]
pub fn install(exe: &Path, dir: &Path) -> Result<Installed, String> {
    let link = dir.join(SHIM_NAME);
    match std::fs::symlink_metadata(&link) {
        Ok(m) if m.file_type().is_symlink() => {
            if std::fs::read_link(&link).is_ok_and(|t| t == exe) {
                return Ok(Installed::AlreadyPresent(link));
            }
            std::fs::remove_file(&link).map_err(|e| format!("{}: {e}", link.display()))?;
        }
        Ok(_) => {
            return Err(format!(
                "{} exists and is not a symlink; remove it first",
                link.display()
            ));
        }
        Err(_) => {}
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    std::os::unix::fs::symlink(exe, &link).map_err(|e| format!("{}: {e}", link.display()))?;
    Ok(Installed::Created(link))
}

#[cfg(not(unix))]
pub fn install(exe: &Path, _dir: &Path) -> Result<Installed, String> {
    Err(format!(
        "Add {} to your PATH to use `fluxwire` from a terminal.",
        exe.parent().unwrap_or(exe).display()
    ))
}

/// Removes the shim; only a symlink is ever deleted. Returns whether one was removed.
pub fn uninstall(dir: &Path) -> Result<bool, String> {
    let link = dir.join(SHIM_NAME);
    match std::fs::symlink_metadata(&link) {
        Ok(m) if m.file_type().is_symlink() => std::fs::remove_file(&link)
            .map(|_| true)
            .map_err(|e| format!("{}: {e}", link.display())),
        Ok(_) => Err(format!("{} is not a symlink; not removed", link.display())),
        Err(_) => Ok(false),
    }
}

impl WorkspaceView {
    pub fn install_cli_shim(&mut self, cx: &mut Context<Self>) {
        let (Some(home), Ok(exe)) = (
            directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()),
            std::env::current_exe(),
        ) else {
            self.notice = Some("Cannot locate the app binary or home directory.".into());
            cx.notify();
            return;
        };
        let dir = shim_dir(&home);
        self.notice = Some(match install(&exe, &dir) {
            Ok(i) => {
                let (Installed::Created(p) | Installed::AlreadyPresent(p)) = i;
                let hint = if on_path(&dir, std::env::var_os("PATH").as_deref()) {
                    String::new()
                } else {
                    format!(" Add {} to your PATH.", dir.display())
                };
                format!("`fluxwire` installed at {}.{hint}", p.display())
            }
            Err(e) => e,
        });
        cx.notify();
    }

    pub fn uninstall_cli_shim(&mut self, cx: &mut Context<Self>) {
        let Some(home) = directories::BaseDirs::new().map(|d| d.home_dir().to_path_buf()) else {
            return;
        };
        self.notice = Some(match uninstall(&shim_dir(&home)) {
            Ok(true) => "Removed the `fluxwire` shell command.".into(),
            Ok(false) => "No `fluxwire` shell command is installed.".into(),
            Err(e) => e,
        });
        cx.notify();
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn tmp(n: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fw-shim-{n}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn install_is_idempotent_and_replaces_stale_links() {
        let d = tmp("a");
        let bin = d.join("bin");
        assert!(matches!(
            install(&d.join("x"), &bin),
            Ok(Installed::Created(_))
        ));
        assert!(matches!(
            install(&d.join("x"), &bin),
            Ok(Installed::AlreadyPresent(_))
        ));
        assert!(matches!(
            install(&d.join("y"), &bin),
            Ok(Installed::Created(_))
        ));
        assert_eq!(
            std::fs::read_link(bin.join(SHIM_NAME)).unwrap(),
            d.join("y")
        );
        assert_eq!(uninstall(&bin), Ok(true));
        assert_eq!(uninstall(&bin), Ok(false));
    }

    #[test]
    fn never_touches_regular_files() {
        let d = tmp("b");
        std::fs::write(d.join(SHIM_NAME), "x").unwrap();
        assert!(install(&d.join("x"), &d).is_err());
        assert!(uninstall(&d).is_err());
        assert!(d.join(SHIM_NAME).is_file());
    }

    #[test]
    fn path_detection() {
        let p = std::env::join_paths(["/a", "/b"]).unwrap();
        assert!(on_path(Path::new("/b"), Some(&p)));
        assert!(!on_path(Path::new("/c"), Some(&p)));
        assert!(!on_path(Path::new("/c"), None));
        assert_eq!(shim_dir(Path::new("/h")), PathBuf::from("/h/.local/bin"));
    }
}
