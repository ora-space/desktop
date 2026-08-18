use crate::WorkspaceFileSystemError;
use crate::workspace::{canonical_root, relative_string};
use notify::event::ModifyKind;
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use ora_utils::path::CanonicalPathRoot;
use std::path::Path;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

/// Identifies the cache invalidation implied by one native filesystem event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkspaceChangeKind {
    Created,
    Modified,
    Removed,
    Renamed { from: String },
    RescanRequired,
}

/// Describes one workspace-relative file change.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkspaceChange {
    pub path: String,
    pub kind: WorkspaceChangeKind,
}

/// Owns one native recursive watcher and batches its platform-specific callback events.
pub struct WorkspaceWatcher {
    _watcher: RecommendedWatcher,
    events: Receiver<notify::Result<Event>>,
    root: CanonicalPathRoot,
}

impl WorkspaceWatcher {
    /// Starts watching one canonical workspace root recursively.
    pub fn start(root: &Path) -> Result<Self, WorkspaceFileSystemError> {
        let root = canonical_root(root)?;
        let (sender, events) = mpsc::channel();
        let mut watcher = notify::recommended_watcher(move |event| {
            let _ = sender.send(event);
        })
        .map_err(|error| WorkspaceFileSystemError::WatchFailed {
            path: root.as_path().to_path_buf(),
            message: error.to_string(),
        })?;
        watcher
            .watch(root.as_path(), RecursiveMode::Recursive)
            .map_err(|error| WorkspaceFileSystemError::WatchFailed {
                path: root.as_path().to_path_buf(),
                message: error.to_string(),
            })?;
        Ok(Self {
            _watcher: watcher,
            events,
            root,
        })
    }

    /// Waits for one event and coalesces follow-up events arriving within the debounce window.
    pub fn receive_batch(
        &self,
        debounce: Duration,
    ) -> Result<Option<Vec<WorkspaceChange>>, WorkspaceFileSystemError> {
        let first = match self.events.recv_timeout(debounce) {
            Ok(event) => event,
            Err(RecvTimeoutError::Timeout) => return Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                return Err(WorkspaceFileSystemError::WatchFailed {
                    path: self.root.as_path().to_path_buf(),
                    message: "native watcher disconnected".to_string(),
                });
            }
        };
        let deadline = Instant::now() + debounce;
        let mut changes = self.map_event(first)?;
        while let Some(remaining) = deadline.checked_duration_since(Instant::now()) {
            match self.events.recv_timeout(remaining) {
                Ok(event) => changes.extend(self.map_event(event)?),
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => break,
            }
        }
        changes.sort_by(|left, right| left.path.cmp(&right.path));
        changes.dedup();
        Ok(Some(changes))
    }

    /// Converts native event shapes into stable workspace-relative invalidation events.
    fn map_event(
        &self,
        event: notify::Result<Event>,
    ) -> Result<Vec<WorkspaceChange>, WorkspaceFileSystemError> {
        let event = event.map_err(|error| WorkspaceFileSystemError::WatchFailed {
            path: self.root.as_path().to_path_buf(),
            message: error.to_string(),
        })?;
        if matches!(event.kind, EventKind::Other | EventKind::Any) {
            return Ok(vec![WorkspaceChange {
                path: String::new(),
                kind: WorkspaceChangeKind::RescanRequired,
            }]);
        }
        if matches!(event.kind, EventKind::Modify(ModifyKind::Name(_))) && event.paths.len() == 2 {
            let from = self.relative_event_path(&event.paths[0])?;
            let path = self.relative_event_path(&event.paths[1])?;
            return Ok(vec![WorkspaceChange {
                path,
                kind: WorkspaceChangeKind::Renamed { from },
            }]);
        }

        let kind = match event.kind {
            EventKind::Create(_) => WorkspaceChangeKind::Created,
            EventKind::Modify(_) | EventKind::Access(_) => WorkspaceChangeKind::Modified,
            EventKind::Remove(_) => WorkspaceChangeKind::Removed,
            EventKind::Other | EventKind::Any => WorkspaceChangeKind::RescanRequired,
        };
        event
            .paths
            .iter()
            .map(|path| {
                Ok(WorkspaceChange {
                    path: self.relative_event_path(path)?,
                    kind: kind.clone(),
                })
            })
            .collect()
    }

    /// Preserves removed paths by relativizing their event spelling without requiring existence.
    fn relative_event_path(&self, path: &Path) -> Result<String, WorkspaceFileSystemError> {
        relative_string(&self.root, path)
    }
}

#[cfg(test)]
mod tests {
    use super::WorkspaceWatcher;
    use pretty_assertions::assert_eq;
    use std::collections::BTreeSet;
    use std::fs;
    use std::time::Duration;
    use tempfile::TempDir;

    /// Verifies native callbacks are normalized to workspace-relative paths.
    #[test]
    fn observes_workspace_file_changes() {
        let workspace =
            TempDir::new().unwrap_or_else(|error| panic!("create temp workspace: {error}"));
        let watcher = WorkspaceWatcher::start(workspace.path())
            .unwrap_or_else(|error| panic!("start workspace watcher: {error}"));

        fs::write(workspace.path().join("watched.txt"), "changed")
            .unwrap_or_else(|error| panic!("write watched fixture: {error}"));
        let changes = watcher
            .receive_batch(Duration::from_secs(2))
            .unwrap_or_else(|error| panic!("receive native file event: {error}"))
            .unwrap_or_else(|| panic!("expected a native file event"));

        assert_eq!(
            changes
                .iter()
                .map(|change| change.path.as_str())
                .collect::<BTreeSet<_>>(),
            BTreeSet::from(["watched.txt"])
        );
    }
}
