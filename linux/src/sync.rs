use std::path::PathBuf;

use anyhow::Result;
use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher, event::ModifyKind};
use tokio::sync::mpsc;

pub struct SyncWatcher {
    _watcher: RecommendedWatcher,
}

/// Start watching `folder` recursively. Returns a receiver that yields absolute
/// paths of files that were created or modified, and a `SyncWatcher` that must
/// be kept alive as long as watching is desired.
pub fn start_sync_watcher(folder: PathBuf) -> Result<(mpsc::UnboundedReceiver<PathBuf>, SyncWatcher)> {
    let (tx, rx) = mpsc::unbounded_channel::<PathBuf>();

    let mut watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
        let Ok(event) = res else { return };
        let relevant = matches!(
            event.kind,
            EventKind::Create(_) | EventKind::Modify(ModifyKind::Data(_))
        );
        if !relevant {
            return;
        }
        for path in event.paths {
            if !path.is_file() {
                continue;
            }
            // Skip hidden files and temp files (e.g. .sessionId.fileId.tmp).
            if path
                .file_name()
                .and_then(|n| n.to_str())
                .map(|n| n.starts_with('.'))
                .unwrap_or(true)
            {
                continue;
            }
            let _ = tx.send(path);
        }
    })?;

    watcher.watch(&folder, RecursiveMode::Recursive)?;
    Ok((rx, SyncWatcher { _watcher: watcher }))
}
