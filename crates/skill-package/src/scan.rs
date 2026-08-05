use crate::path::RelativePath;
use crate::snapshot::{Snapshot, SnapshotFile};
use std::collections::{BTreeMap, BTreeSet};

/// One discovered, non-overlapping skill root and the files it owns.
///
/// Ownership follows the nearest-manifest rule: every file belongs to the deepest ancestor
/// directory that contains an exact `SKILL.md`, so nested child skills cut off their parent's
/// subtree even when their manifest is invalid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SkillBoundary {
    /// Validated relative path of the boundary's own `SKILL.md`.
    pub manifest_path: RelativePath,
    /// Ordinary files owned by this boundary, sorted by relative path.
    pub files: Vec<SnapshotFile>,
}

impl SkillBoundary {
    /// Returns the number of ordinary files owned by this boundary.
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// Returns the cumulative ordinary bytes owned by this boundary.
    pub fn total_size(&self) -> u64 {
        self.files.iter().map(|file| file.size).sum()
    }
}

/// Discovers non-overlapping skill boundaries from one validated snapshot.
///
/// Files that do not belong to any manifest root are ignored. The returned boundaries are
/// sorted by manifest path so preview and commit order stay deterministic.
pub fn scan_skill_boundaries(snapshot: &Snapshot) -> Vec<SkillBoundary> {
    let mut manifest_roots = BTreeSet::new();
    for file in snapshot.files() {
        if file.relative_path.is_manifest()
            && let Some(root) = file.relative_path.parent()
        {
            manifest_roots.insert(root);
        }
    }

    let mut grouped: BTreeMap<RelativePath, Vec<SnapshotFile>> = BTreeMap::new();
    for file in snapshot.files() {
        if file.relative_path.is_manifest()
            && let Some(root) = file.relative_path.parent()
        {
            grouped.entry(root).or_default().push(file.clone());
            continue;
        }
        if let Some(root) = nearest_manifest_root(file.relative_path.parent(), &manifest_roots) {
            grouped.entry(root).or_default().push(file.clone());
        }
    }

    grouped
        .into_iter()
        .map(|(root, mut files)| {
            files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
            SkillBoundary {
                manifest_path: root.append_segment("SKILL.md"),
                files,
            }
        })
        .collect()
}

/// Returns the deepest ancestor directory that owns a manifest, if any.
fn nearest_manifest_root(
    mut directory: Option<RelativePath>,
    manifest_roots: &BTreeSet<RelativePath>,
) -> Option<RelativePath> {
    while let Some(current) = directory {
        if manifest_roots.contains(&current) {
            return Some(current);
        }
        directory = current.parent();
    }
    None
}
