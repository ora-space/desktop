//! One-off upgrade migration code for databases upgrading from pre-`0025` schemas.
//!
//! Migration `0025` irreversibly retires the Effect ledger tables and switches Skill delivery from
//! workspace materialization to formal storage paths. This module performs two pre-`0025` upgrade
//! steps strictly before migrations run:
//! 1. Backing up an existing pre-`0025` SQLite database to `<db>.pre-0025.bak` so users who need
//!    to revert to an older Desktop build have a self-contained recovery copy.
//! 2. Removing verified, unedited legacy Effect-managed skill directories while Effect rows still
//!    exist in SQLite.
//!
//! Both entry points are no-ops for fresh installations and post-`0025` databases. This module is
//! safe to remove once no supported upgrade path starts before schema `0025`.

#[cfg(test)]
mod tests;

use crate::DatabaseError;
use ora_logging::{ora_info, ora_warn};
use ora_utils::directory::fingerprint_directory;
use ora_utils::path::{PortableRelativePath, StrictRelativePath};
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::time::Duration;

const MARKER_FILE_NAME: &str = ".ora-managed.json";
const MARKER_SCHEMA_VERSION: u32 = 1;
const PRE_0025_BACKUP_SUFFIX: &str = ".pre-0025.bak";
const PRE_0025_BACKUP_TEMP_SUFFIX: &str = ".pre-0025.bak.tmp";

/// Returns the version-marked backup path written beside `database_path` before migration `0025`.
pub fn pre_0025_backup_path(database_path: &Path) -> PathBuf {
    append_path_suffix(database_path, PRE_0025_BACKUP_SUFFIX)
}

/// Creates a self-contained SQLite backup beside `database_path` before irreversible migration
/// `0025` runs on an existing pre-`0025` database.
///
/// Uses `VACUUM INTO` to fold committed WAL frames into a standalone sibling file and atomically
/// renames it into place so partial backups are never left behind. Missing databases,
/// uninitialized empty files, and databases that already applied `0025` return `Ok(None)`. If the
/// version-marked backup already exists (for example after an interrupted prior upgrade attempt),
/// it is preserved untouched and returned as `Ok(Some(path))`.
pub fn backup_database_before_0025(database_path: &Path) -> Result<Option<PathBuf>, DatabaseError> {
    if !database_path.exists() {
        return Ok(None);
    }
    let connection = Connection::open(database_path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    if !needs_pre_0025_backup(&connection)? {
        return Ok(None);
    }

    let backup_path = pre_0025_backup_path(database_path);
    if backup_path.exists() {
        return Ok(Some(backup_path));
    }

    let temp_path = append_path_suffix(database_path, PRE_0025_BACKUP_TEMP_SUFFIX);
    if let Err(error) = fs::remove_file(&temp_path)
        && error.kind() != ErrorKind::NotFound
    {
        return Err(DatabaseError::PreMigrationBackupFailed {
            version: "0025",
            path: temp_path,
            message: format!("failed to clear stale temporary backup: {error}"),
        });
    }
    let temp_path_str =
        temp_path
            .to_str()
            .ok_or_else(|| DatabaseError::PreMigrationBackupFailed {
                version: "0025",
                path: temp_path.clone(),
                message: "backup path is not valid UTF-8".to_string(),
            })?;
    if let Err(error) = connection.execute("VACUUM INTO ?1", [temp_path_str]) {
        let _ = fs::remove_file(&temp_path);
        return Err(DatabaseError::PreMigrationBackupFailed {
            version: "0025",
            path: backup_path,
            message: format!("SQLite VACUUM INTO failed: {error}"),
        });
    }
    drop(connection);

    if let Err(error) = fs::rename(&temp_path, &backup_path) {
        let _ = fs::remove_file(&temp_path);
        return Err(DatabaseError::PreMigrationBackupFailed {
            version: "0025",
            path: backup_path,
            message: format!("failed to finalize backup file: {error}"),
        });
    }

    ora_info!(
        backup_path = %backup_path.display(),
        "created pre-0025 SQLite backup before irreversible Effect retirement migration"
    );
    Ok(Some(backup_path))
}

fn needs_pre_0025_backup(connection: &Connection) -> Result<bool, DatabaseError> {
    if !has_table(connection, "migrations")? {
        return Ok(false);
    }
    let applied_count: i64 =
        connection.query_row("SELECT COUNT(*) FROM migrations", [], |row| row.get(0))?;
    if applied_count == 0 {
        return Ok(false);
    }
    let has_0025: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM migrations WHERE version = '0025' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(has_0025.is_none())
}

fn append_path_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut os = OsString::from(path.as_os_str());
    os.push(suffix);
    PathBuf::from(os)
}

/// Persisted legacy `ora/skill-directory.v1` record read before migration `0025` drops Effect tables.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LegacyManagedSkillRow {
    pub managed_identity: String,
    pub resource_id: String,
    pub native_identity: String,
    pub fingerprint: String,
    pub descriptor_json: String,
}

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
/// or junctions are skipped cleanly so user work is preserved and startup never blocks. Shared by
/// both packaged Desktop bootstrap and development migration reconciliation (`xtask`).
pub fn cleanup_legacy_managed_skills(database_path: &Path) {
    let rows = match read_legacy_managed_skills(database_path) {
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

/// Reads legacy Effect-managed skill directory rows when Effect tables still exist on disk.
///
/// Missing databases and post-`0025` databases return an empty list without creating files or
/// mutating state so startup can run cleanup strictly before schema migrations execute.
pub fn read_legacy_managed_skills(
    database_path: &Path,
) -> Result<Vec<LegacyManagedSkillRow>, DatabaseError> {
    if !database_path.exists() {
        return Ok(Vec::new());
    }
    let connection = Connection::open(database_path)?;
    connection.busy_timeout(Duration::from_secs(5))?;
    if !has_table(&connection, "effect_managed_items")?
        || !has_table(&connection, "effect_resources")?
    {
        return Ok(Vec::new());
    }
    let mut statement = connection.prepare(
        "SELECT item.id, item.resource_id, item.native_identity, item.fingerprint, resource.descriptor_json
         FROM effect_managed_items item
         JOIN effect_resources resource ON resource.id = item.resource_id
         WHERE resource.adapter_id = 'ora/filesystem-directory'
           AND resource.materialization_format = 'ora/skill-directory.v1'
         ORDER BY item.resource_id, item.native_identity, item.id",
    )?;
    let rows = statement
        .query_map([], |row| {
            Ok(LegacyManagedSkillRow {
                managed_identity: row.get(0)?,
                resource_id: row.get(1)?,
                native_identity: row.get(2)?,
                fingerprint: row.get(3)?,
                descriptor_json: row.get(4)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

fn has_table(connection: &Connection, table_name: &str) -> Result<bool, DatabaseError> {
    let found: Option<i64> = connection
        .query_row(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1 LIMIT 1",
            [table_name],
            |row| row.get(0),
        )
        .optional()?;
    Ok(found.is_some())
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
