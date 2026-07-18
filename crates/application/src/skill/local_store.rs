use crate::skill::ports::{SkillPackageStore, SkillPackageStoreError};
use crate::skill::validation::{SKILL_MANIFEST_FILE, is_safe_skill_name};
use ora_domain::SkillId;
use std::fs;
use std::io::ErrorKind;
use std::path::{Component, Path, PathBuf};

/// Marks in-progress staging directories so reconciliation can tell them from committed skills.
const STAGING_SUFFIX: &str = ".tmp";

/// Stores uploaded skill folders under a single `atoms/skills` root on the local filesystem.
///
/// The adapter is `Clone` because the same layout is shared by the import, delete, and startup
/// reconciliation use cases; cloning only copies the root path, not any open handles.
#[derive(Clone, Debug)]
pub struct LocalSkillPackageStore {
    root: PathBuf,
}

impl LocalSkillPackageStore {
    /// Builds the store rooted at the configured `atoms/skills` directory.
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// Returns the `<id>.tmp` staging directory path for an in-progress import.
    fn staging_path(&self, skill_id: &SkillId) -> PathBuf {
        self.root.join(format!("{skill_id}{STAGING_SUFFIX}"))
    }

    /// Returns the committed `<name>` directory path for a resolved skill.
    fn committed_path(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }
}

impl SkillPackageStore for LocalSkillPackageStore {
    fn create_staging(&self, skill_id: &SkillId) -> Result<(), SkillPackageStoreError> {
        fs::create_dir_all(self.staging_path(skill_id)).map_err(operation_failed)
    }

    fn write_file(
        &self,
        skill_id: &SkillId,
        relative_path: &Path,
        bytes: &[u8],
    ) -> Result<(), SkillPackageStoreError> {
        // Defense in depth at the filesystem boundary: the caller normalizes paths, but re-reject
        // any non-`Normal` component so a staged write can never escape the staging directory.
        if relative_path
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
        {
            return Err(SkillPackageStoreError::OperationFailed(format!(
                "refusing to stage unsafe path `{}`",
                relative_path.display()
            )));
        }

        let target = self.staging_path(skill_id).join(relative_path);
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(operation_failed)?;
        }
        fs::write(target, bytes).map_err(operation_failed)
    }

    fn read_manifest(&self, skill_id: &SkillId) -> Result<Option<String>, SkillPackageStoreError> {
        let manifest_path = self.staging_path(skill_id).join(SKILL_MANIFEST_FILE);
        match fs::read_to_string(manifest_path) {
            Ok(contents) => Ok(Some(contents)),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(None),
            Err(error) => Err(operation_failed(error)),
        }
    }

    fn committed_exists(&self, name: &str) -> Result<bool, SkillPackageStoreError> {
        Ok(is_safe_skill_name(name) && self.committed_path(name).is_dir())
    }

    fn promote(&self, skill_id: &SkillId, name: &str) -> Result<(), SkillPackageStoreError> {
        let destination = self.committed_path(name);
        // Never overwrite an existing committed skill: a losing concurrent import must fail rather
        // than clobber the directory the winning import already promoted.
        if destination.exists() {
            return Err(SkillPackageStoreError::OperationFailed(format!(
                "committed skill directory `{name}` already exists"
            )));
        }
        fs::rename(self.staging_path(skill_id), destination).map_err(operation_failed)
    }

    fn discard_staging(&self, skill_id: &SkillId) -> Result<(), SkillPackageStoreError> {
        remove_dir_if_present(&self.staging_path(skill_id))
    }

    fn remove_committed(&self, name: &str) -> Result<(), SkillPackageStoreError> {
        // Only ever touch a safe single-segment child; names from the plain JSON create path may be
        // arbitrary strings and never have a directory, so unsafe names are a no-op by design.
        if !is_safe_skill_name(name) {
            return Ok(());
        }
        remove_dir_if_present(&self.committed_path(name))
    }

    fn remove_all_staging(&self) -> Result<(), SkillPackageStoreError> {
        for entry in read_dir_if_present(&self.root)? {
            if entry.name.ends_with(STAGING_SUFFIX) && entry.is_dir {
                remove_dir_if_present(&self.root.join(&entry.name))?;
            }
        }
        Ok(())
    }

    fn list_committed_names(&self) -> Result<Vec<String>, SkillPackageStoreError> {
        let mut names = Vec::new();
        for entry in read_dir_if_present(&self.root)? {
            if entry.is_dir && !entry.name.ends_with(STAGING_SUFFIX) {
                names.push(entry.name);
            }
        }
        Ok(names)
    }
}

/// Describes one direct child of the skills root captured for reconciliation decisions.
struct DirectoryEntry {
    name: String,
    is_dir: bool,
}

/// Lists direct children of the skills root, treating an absent root as an empty directory.
fn read_dir_if_present(root: &Path) -> Result<Vec<DirectoryEntry>, SkillPackageStoreError> {
    let read_dir = match fs::read_dir(root) {
        Ok(read_dir) => read_dir,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(operation_failed(error)),
    };

    let mut entries = Vec::new();
    for entry in read_dir {
        let entry = entry.map_err(operation_failed)?;
        let file_type = entry.file_type().map_err(operation_failed)?;
        entries.push(DirectoryEntry {
            name: entry.file_name().to_string_lossy().into_owned(),
            is_dir: file_type.is_dir(),
        });
    }

    Ok(entries)
}

/// Removes a directory tree, treating an already-absent directory as success.
fn remove_dir_if_present(path: &Path) -> Result<(), SkillPackageStoreError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(operation_failed(error)),
    }
}

/// Wraps a filesystem failure into the stable skill storage error.
fn operation_failed(error: std::io::Error) -> SkillPackageStoreError {
    SkillPackageStoreError::OperationFailed(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::LocalSkillPackageStore;
    use crate::skill::ports::SkillPackageStore;
    use ora_domain::SkillId;
    use pretty_assertions::assert_eq;
    use std::fs;
    use tempfile::TempDir;

    const MANIFEST: &str = "---\nname: grilling\ndescription: Grill the user\n---\n";

    #[test]
    fn stages_files_then_promotes_them_into_a_committed_directory() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("atoms").join("skills");
        fs::create_dir_all(&root).unwrap();
        let store = LocalSkillPackageStore::new(root.clone());
        let skill_id = SkillId::new("id-1");

        store.create_staging(&skill_id).unwrap();
        store
            .write_file(
                &skill_id,
                std::path::Path::new("SKILL.md"),
                MANIFEST.as_bytes(),
            )
            .unwrap();
        store
            .write_file(
                &skill_id,
                &std::path::Path::new("refs").join("util.py"),
                b"noop",
            )
            .unwrap();

        assert_eq!(
            store.read_manifest(&skill_id).unwrap(),
            Some(MANIFEST.to_string())
        );
        assert_eq!(store.committed_exists("grilling").unwrap(), false);

        store.promote(&skill_id, "grilling").unwrap();

        assert_eq!(store.committed_exists("grilling").unwrap(), true);
        assert_eq!(
            fs::read_to_string(root.join("grilling").join("SKILL.md")).unwrap(),
            MANIFEST
        );
        assert_eq!(
            fs::read_to_string(root.join("grilling").join("refs").join("util.py")).unwrap(),
            "noop"
        );
        assert_eq!(root.join("id-1.tmp").exists(), false);
        assert_eq!(
            store.list_committed_names().unwrap(),
            vec!["grilling".to_string()]
        );
    }

    #[test]
    fn promote_refuses_to_overwrite_an_existing_committed_directory() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("atoms").join("skills");
        fs::create_dir_all(&root).unwrap();
        let store = LocalSkillPackageStore::new(root);
        let occupant = SkillId::new("id-1");
        store.create_staging(&occupant).unwrap();
        store
            .write_file(
                &occupant,
                std::path::Path::new("SKILL.md"),
                MANIFEST.as_bytes(),
            )
            .unwrap();
        store.promote(&occupant, "grilling").unwrap();

        let contender = SkillId::new("id-2");
        store.create_staging(&contender).unwrap();
        store
            .write_file(
                &contender,
                std::path::Path::new("SKILL.md"),
                MANIFEST.as_bytes(),
            )
            .unwrap();

        assert_eq!(
            store.promote(&contender, "grilling"),
            Err(
                crate::skill::ports::SkillPackageStoreError::OperationFailed(
                    "committed skill directory `grilling` already exists".to_string(),
                )
            )
        );
    }

    #[test]
    fn removes_only_staging_directories_during_sweep() {
        let temp_dir = TempDir::new().unwrap();
        let root = temp_dir.path().join("atoms").join("skills");
        fs::create_dir_all(&root).unwrap();
        let store = LocalSkillPackageStore::new(root.clone());
        let promoted = SkillId::new("id-1");
        store.create_staging(&promoted).unwrap();
        store
            .write_file(
                &promoted,
                std::path::Path::new("SKILL.md"),
                MANIFEST.as_bytes(),
            )
            .unwrap();
        store.promote(&promoted, "grilling").unwrap();
        store.create_staging(&SkillId::new("id-abandoned")).unwrap();

        store.remove_all_staging().unwrap();

        assert_eq!(root.join("id-abandoned.tmp").exists(), false);
        assert_eq!(
            store.list_committed_names().unwrap(),
            vec!["grilling".to_string()]
        );

        store.remove_committed("grilling").unwrap();
        assert_eq!(store.committed_exists("grilling").unwrap(), false);
    }
}
