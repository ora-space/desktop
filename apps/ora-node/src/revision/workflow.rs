//! Whole-run archival preserves raw session bytes alongside a versioned manifest.
use super::prepare::{copy_synced, stored};
use ora_node_protocol::*;
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
};

/// Stable local names never interpret Cloud's object keys as filesystem paths.
pub(crate) fn object_file(key: &ObjectKey) -> String {
    ora_utils::hash::sha256_hex(key.as_str().as_bytes())
}

/// Freezes every established history before the Git snapshot or any external mutation.
pub(super) fn freeze_history(
    directory: &Path,
    manifest_path: &Path,
    spec: &DeliverRevisionSpec,
    sources: &[(WorkflowSessionAttempt, Option<PathBuf>)],
) -> Result<Vec<StoredObject>, RevisionFailureCode> {
    let input = spec
        .workflow
        .as_ref()
        .ok_or(RevisionFailureCode::HistoryUnavailable)?;
    if sources
        .iter()
        .map(|(attempt, _)| attempt)
        .ne(input.sessions.iter())
    {
        return Err(RevisionFailureCode::HistoryUnavailable);
    }
    let mut objects = Vec::new();
    let mut sessions = Vec::new();
    for (attempt, source) in sources {
        let history = if let Some(source) = source {
            let destination = directory.join(object_file(&attempt.history_key));
            copy_synced(source, &destination)
                .map_err(|_| RevisionFailureCode::HistoryUnavailable)?;
            let object = stored(&attempt.history_key, &destination)
                .map_err(|_| RevisionFailureCode::HistoryUnavailable)?;
            objects.push(object.clone());
            WorkflowSessionHistory::Archived { object }
        } else {
            WorkflowSessionHistory::NotStarted
        };
        sessions.push(WorkflowHistoryEntry {
            execution_order: attempt.execution_order,
            node_id: attempt.node_id.clone(),
            attempt: attempt.attempt,
            execution_id: attempt.execution_id.clone(),
            history,
        });
    }
    let manifest = WorkflowHistoryManifest {
        schema_version: 1,
        workflow_run_id: input.workflow_run_id.clone(),
        checkout_execution_id: spec.checkout_execution_id.clone(),
        sessions,
    };
    let bytes =
        serde_json::to_vec(&manifest).map_err(|_| RevisionFailureCode::HistoryUnavailable)?;
    let mut file = fs::OpenOptions::new()
        .write(/*write*/ true)
        .create_new(/*create_new*/ true)
        .open(manifest_path)
        .map_err(|_| RevisionFailureCode::HistoryUnavailable)?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| RevisionFailureCode::HistoryUnavailable)?;
    Ok(objects)
}
