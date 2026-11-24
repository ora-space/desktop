#[cfg(test)]
mod tests;

use ora_db::LegacyManagedSkillRow;
use ora_logging::ora_warn;
use ora_utils::directory::fingerprint_directory;
use ora_utils::path::{PortableRelativePath, StrictRelativePath};
use serde::Deserialize;
use std::ffi::OsStr;
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

const MARKER_FILE_NAME: &str = ".ora-managed.json";
const MARKER_SCHEMA_VERSION: u32 = 1;

#[derive(Deserialize)]
struct LegacyResourceDescriptorEnvelope {
    kind: String,
    #[serde(default)]
    descriptor: serde_json::Value,
}

#[derive(Deserialize)]
struct FilesystemDirectoryDescriptor {
    workspace_root: PathBuf,
    relative_path: String,
}

#[derive(Deserialize)]
struct LegacyManagedMarker {
    schema_version: u32,
    resource: String,
    managed_identity: String,
}

/// Removes unedited legacy Effect-managed skill directories before schema `0025` drops Effect tables.
///
/// Missing databases, post-`0025` schemas, drifted files, missing/mismatched markers, and symlinks
/// or junctions are skipped cleanly so user work is preserved and startup never blocks.
pub(crate) fn cleanup_legacy_managed_skills(database_path: &Path) {
    let rows = match ora_db::read_legacy_managed_skills(database_path) {
        Ok(rows) => rows,
        Err(error) => {
            ora_warn!(
                error = %error,
                "failed to query legacy Effect managed skill directories; skipping cleanup"
            );
            return;
        }
    };

    for row in rows {
        if let Err(reason) = cleanup_one_managed_skill(&row) {
            ora_warn!(
                resource_id = %row.resource_id,
                managed_identity = %row.managed_identity,
                relative_path = %sanitized_display_path(&row),
                reason = %reason,
                "preserved legacy Effect skill directory during pre-migration cleanup"
            );
        }
    }
}

fn cleanup_one_managed_skill(row: &LegacyManagedSkillRow) -> Result<(), String> {
    let descriptor = parse_directory_descriptor(&row.descriptor_json)?;
    if !descriptor.workspace_root.is_absolute() {
        return Err("resource workspace_root is not absolute".to_string());
    }
    let relative_path = PortableRelativePath::parse(&descriptor.relative_path)
        .map_err(|error| format!("invalid resource relative_path: {error}"))?;
    if relative_path.is_root() {
        return Err("resource relative_path must not be workspace root".to_string());
    }
    validate_native_identity(&row.native_identity)?;

    let workspace_meta = match fs::symlink_metadata(&descriptor.workspace_root) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to stat workspace_root: {error}")),
    };
    if workspace_meta.file_type().is_symlink() || !workspace_meta.is_dir() {
        return Err("workspace_root is a link or not a directory".to_string());
    }
    let canonical_workspace = fs::canonicalize(&descriptor.workspace_root)
        .map_err(|error| format!("failed to canonicalize workspace_root: {error}"))?;

    let mut current = descriptor.workspace_root;
    for segment in relative_path.to_path_buf().components() {
        current.push(segment.as_os_str());
        let meta = match fs::symlink_metadata(&current) {
            Ok(meta) => meta,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(format!("failed to stat resource ancestor: {error}")),
        };
        if meta.file_type().is_symlink() || !meta.is_dir() {
            return Err(
                "resource path traverses a symlink, junction, or non-directory".to_string(),
            );
        }
        let canonical = fs::canonicalize(&current)
            .map_err(|error| format!("failed to canonicalize resource path: {error}"))?;
        if !canonical.starts_with(&canonical_workspace) {
            return Err("resource path resolves outside workspace_root".to_string());
        }
    }

    let skill_dir = current.join(&row.native_identity);
    let skill_meta = match fs::symlink_metadata(&skill_dir) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("failed to stat managed skill directory: {error}")),
    };
    if skill_meta.file_type().is_symlink() {
        return Err("managed skill directory is a symlink or junction".to_string());
    }
    if !skill_meta.is_dir() {
        return Err("managed skill path is not a directory".to_string());
    }
    let canonical_skill_dir = fs::canonicalize(&skill_dir)
        .map_err(|error| format!("failed to canonicalize managed skill directory: {error}"))?;
    if !canonical_skill_dir.starts_with(&canonical_workspace) {
        return Err("managed skill directory resolves outside workspace_root".to_string());
    }

    verify_marker(&skill_dir, row)?;
    verify_fingerprint(&skill_dir, &row.fingerprint)?;

    fs::remove_dir_all(&skill_dir)
        .map_err(|error| format!("failed to delete legacy managed skill directory: {error}"))?;
    Ok(())
}

fn parse_directory_descriptor(
    descriptor_json: &str,
) -> Result<FilesystemDirectoryDescriptor, String> {
    let envelope: LegacyResourceDescriptorEnvelope = serde_json::from_str(descriptor_json)
        .map_err(|error| format!("malformed resource descriptor JSON: {error}"))?;
    if envelope.kind != "filesystem_directory_v1" {
        return Err(format!(
            "unsupported resource descriptor kind: {}",
            envelope.kind
        ));
    }
    serde_json::from_value(envelope.descriptor)
        .map_err(|error| format!("invalid filesystem_directory_v1 descriptor: {error}"))
}

fn validate_native_identity(native_identity: &str) -> Result<(), String> {
    if native_identity.contains('/') || native_identity.contains('\\') {
        return Err("native_identity must be a single directory segment".to_string());
    }
    StrictRelativePath::parse(native_identity)
        .map_err(|error| format!("unsafe native_identity: {error:?}"))?;
    Ok(())
}

fn verify_marker(skill_dir: &Path, row: &LegacyManagedSkillRow) -> Result<(), String> {
    let marker_path = skill_dir.join(MARKER_FILE_NAME);
    let meta = match fs::symlink_metadata(&marker_path) {
        Ok(meta) => meta,
        Err(error) if error.kind() == ErrorKind::NotFound => {
            return Err("missing .ora-managed.json marker".to_string());
        }
        Err(error) => return Err(format!("failed to stat .ora-managed.json: {error}")),
    };
    if meta.file_type().is_symlink() || !meta.is_file() {
        return Err(".ora-managed.json marker is a link or not a regular file".to_string());
    }
    let bytes = fs::read(&marker_path)
        .map_err(|error| format!("failed to read .ora-managed.json: {error}"))?;
    let marker: LegacyManagedMarker = serde_json::from_slice(&bytes)
        .map_err(|error| format!("malformed .ora-managed.json marker: {error}"))?;
    if marker.schema_version != MARKER_SCHEMA_VERSION {
        return Err(format!(
            "unsupported .ora-managed.json schema_version {}",
            marker.schema_version
        ));
    }
    if marker.resource != row.resource_id {
        return Err("marker resource id does not match database record".to_string());
    }
    if marker.managed_identity != row.managed_identity {
        return Err("marker managed_identity does not match database record".to_string());
    }
    Ok(())
}

fn verify_fingerprint(skill_dir: &Path, expected_fingerprint: &str) -> Result<(), String> {
    let computed = fingerprint_directory(skill_dir, &[OsStr::new(MARKER_FILE_NAME)])
        .map_err(|error| format!("failed to fingerprint legacy skill directory: {error}"))?;
    if computed.to_string() != expected_fingerprint {
        return Err("legacy skill directory contents differ from recorded fingerprint".to_string());
    }
    Ok(())
}

fn sanitized_display_path(row: &LegacyManagedSkillRow) -> String {
    let raw = serde_json::from_str::<LegacyResourceDescriptorEnvelope>(&row.descriptor_json)
        .ok()
        .and_then(|envelope| {
            serde_json::from_value::<FilesystemDirectoryDescriptor>(envelope.descriptor).ok()
        })
        .map(|descriptor| format!("{}/{}", descriptor.relative_path, row.native_identity))
        .unwrap_or_else(|| row.native_identity.clone());
    raw.chars()
        .map(|ch| if ch.is_control() { '\u{fffd}' } else { ch })
        .collect()
}
