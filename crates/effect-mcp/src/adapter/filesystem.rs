use super::{LEDGER_SCHEMA_VERSION, MAX_CONFIG_BYTES, McpAdapterError, McpOwnershipLedger};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

/// Refuses links and non-directory ancestors before a shared configuration path is accessed.
pub(super) fn ensure_contained_file_path(
    workspace_root: &Path,
    candidate: &Path,
) -> Result<(), McpAdapterError> {
    let root_metadata =
        fs::symlink_metadata(workspace_root).map_err(|source| McpAdapterError::Io {
            path: workspace_root.to_path_buf(),
            source,
        })?;
    if !root_metadata.is_dir() || root_metadata.file_type().is_symlink() {
        return Err(McpAdapterError::UnsafeOperationPath);
    }
    let relative = candidate
        .strip_prefix(workspace_root)
        .map_err(|_| McpAdapterError::UnsafeOperationPath)?;
    let canonical_workspace =
        workspace_root
            .canonicalize()
            .map_err(|source| McpAdapterError::Io {
                path: workspace_root.to_path_buf(),
                source,
            })?;
    let mut current = canonical_workspace.clone();
    let mut components = relative.components().peekable();
    while let Some(component) = components.next() {
        let std::path::Component::Normal(segment) = component else {
            return Err(McpAdapterError::UnsafeOperationPath);
        };
        current = current.join(segment);
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                let is_file = components.peek().is_none();
                if metadata.file_type().is_symlink()
                    || (is_file && !metadata.is_file())
                    || (!is_file && !metadata.is_dir())
                {
                    return Err(McpAdapterError::UnsafeOperationPath);
                }
                let canonical = current
                    .canonicalize()
                    .map_err(|source| McpAdapterError::Io {
                        path: current.clone(),
                        source,
                    })?;
                if !canonical.starts_with(&canonical_workspace) {
                    return Err(McpAdapterError::UnsafeOperationPath);
                }
                current = canonical;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                // Missing suffixes are safe because ResourcePath already rejects traversal; apply
                // creates them only below the last verified real directory.
                break;
            }
            Err(source) => {
                return Err(McpAdapterError::Io {
                    path: current,
                    source,
                });
            }
        }
    }
    Ok(())
}

/// Reads one sidecar or returns a format-bound empty ledger.
pub(super) fn read_ledger(
    path: &Path,
    format: &str,
) -> Result<McpOwnershipLedger, McpAdapterError> {
    let Some(source) = read_optional_bounded(path)? else {
        return Ok(McpOwnershipLedger {
            schema_version: LEDGER_SCHEMA_VERSION,
            materialization_format: format.to_string(),
            managed: BTreeMap::new(),
        });
    };
    let ledger: McpOwnershipLedger = serde_json::from_str(&source)?;
    if ledger.schema_version != LEDGER_SCHEMA_VERSION || ledger.materialization_format != format {
        return Err(McpAdapterError::OwnershipMismatch);
    }
    Ok(ledger)
}

/// Reads a bounded UTF-8 file without creating missing paths.
pub(super) fn read_optional_bounded(path: &Path) -> Result<Option<String>, McpAdapterError> {
    let metadata = match fs::metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(source) => {
            return Err(McpAdapterError::Io {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    if metadata.len() > MAX_CONFIG_BYTES {
        return Err(McpAdapterError::TooLarge(path.to_path_buf()));
    }
    fs::read_to_string(path)
        .map(Some)
        .map_err(|source| McpAdapterError::Io {
            path: path.to_path_buf(),
            source,
        })
}
