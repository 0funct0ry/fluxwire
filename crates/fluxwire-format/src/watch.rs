//! File watching with a 100 ms debounce (SPEC §5.3).

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Duration;

pub const DEBOUNCE: Duration = Duration::from_millis(100);

/// Keeps the watcher alive; dropping it stops watching.
pub struct WatchHandle {
    _watcher: RecommendedWatcher,
}

/// Watch `root` recursively. Batches of changed paths (deduplicated) are sent on `tx` after
/// `DEBOUNCE` of quiet. Paths inside `.git` and `.fluxwire/cache` are ignored.
pub fn watch(root: &Path, tx: mpsc::Sender<Vec<PathBuf>>) -> notify::Result<WatchHandle> {
    let (raw_tx, raw_rx) = mpsc::channel::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        if let Ok(ev) = res {
            for p in ev.paths {
                let s = p.to_string_lossy();
                if s.contains("/.git/") || s.contains("\\.git\\") || s.contains(".fluxwire/cache") {
                    continue;
                }
                let _ = raw_tx.send(p);
            }
        }
    })?;
    watcher.watch(root, RecursiveMode::Recursive)?;
    std::thread::spawn(move || {
        while let Ok(first) = raw_rx.recv() {
            let mut batch = BTreeSet::from([first]);
            while let Ok(p) = raw_rx.recv_timeout(DEBOUNCE) {
                batch.insert(p);
            }
            if tx.send(batch.into_iter().collect()).is_err() {
                break;
            }
        }
    });
    Ok(WatchHandle { _watcher: watcher })
}
