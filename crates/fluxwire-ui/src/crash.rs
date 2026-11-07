//! Local crash logs (SPEC §21, M14): a panic hook writes `crash-<unix time>.log` under
//! `<config dir>/crashes/`; the next launch offers to open the newest unseen one. Nothing is
//! uploaded. The log holds the panic message, location, a backtrace, the version and the OS —
//! never request data. File handling is plain functions, tested with temp directories.

use std::path::{Path, PathBuf};

const SEEN_MARKER: &str = ".seen";

pub fn crash_dir(config_dir: &Path) -> PathBuf {
    config_dir.join("crashes")
}

pub fn log_name(unix_secs: u64) -> String {
    format!("crash-{unix_secs}.log")
}

/// Text of a log for a panic.
pub fn format_log(version: &str, message: &str, location: &str, backtrace: &str) -> String {
    format!(
        "Fluxwire {version} crashed ({os}/{arch}).\n\nPanic: {message}\nLocation: {location}\n\nBacktrace:\n{backtrace}\n",
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
    )
}

pub fn write_log(dir: &Path, unix_secs: u64, text: &str) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let p = dir.join(log_name(unix_secs));
    std::fs::write(&p, text)?;
    Ok(p)
}

fn log_time(p: &Path) -> Option<u64> {
    p.file_name()?
        .to_str()?
        .strip_prefix("crash-")?
        .strip_suffix(".log")?
        .parse()
        .ok()
}

/// The newest log newer than the `.seen` marker, if any.
pub fn newest_unseen(dir: &Path) -> Option<PathBuf> {
    let seen: u64 = std::fs::read_to_string(dir.join(SEEN_MARKER))
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0);
    std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter_map(|p| log_time(&p).map(|t| (t, p)))
        .filter(|(t, _)| *t > seen)
        .max_by_key(|(t, _)| *t)
        .map(|(_, p)| p)
}

/// Marks everything up to and including `log` as seen.
pub fn mark_seen(dir: &Path, log: &Path) -> std::io::Result<()> {
    match log_time(log) {
        Some(t) => std::fs::write(dir.join(SEEN_MARKER), t.to_string()),
        None => Ok(()),
    }
}

/// Installs the panic hook; the default hook still runs afterwards.
pub fn install_hook(config_dir: PathBuf) {
    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "(non-text panic payload)".into());
        let location = info
            .location()
            .map_or("unknown".into(), |l| format!("{}:{}", l.file(), l.line()));
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let text = format_log(env!("CARGO_PKG_VERSION"), &message, &location, &bt);
        let _ = write_log(&crash_dir(&config_dir), now, &text);
        prev(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("fw_crash_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn log_has_context_but_no_request_data() {
        let t = format_log("1.0.0", "boom", "a.rs:1", "frames");
        assert!(t.contains("1.0.0") && t.contains("boom") && t.contains("a.rs:1"));
        assert!(t.contains(std::env::consts::OS));
    }

    #[test]
    fn newest_unseen_respects_the_marker() {
        let d = tmp("seen").join("crashes");
        assert!(newest_unseen(&d).is_none());
        write_log(&d, 100, "a").unwrap();
        let b = write_log(&d, 200, "b").unwrap();
        assert_eq!(newest_unseen(&d), Some(b.clone()));
        mark_seen(&d, &b).unwrap();
        assert!(newest_unseen(&d).is_none());
        let c = write_log(&d, 300, "c").unwrap();
        assert_eq!(newest_unseen(&d), Some(c));
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }

    #[test]
    fn ignores_foreign_files() {
        let d = tmp("foreign").join("crashes");
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("notes.txt"), "x").unwrap();
        assert!(newest_unseen(&d).is_none());
        let _ = std::fs::remove_dir_all(d.parent().unwrap());
    }
}
