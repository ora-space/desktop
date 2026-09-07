//! Cargo target ownership and module reachability for the Rust architecture check.

use super::{ModuleSize, Result};
use ora_utils::path::CanonicalPathRoot;
use ora_utils::rust_source::{
    RustSourceAnalysis, RustSourceKind, RustSourceUse, analyze_rust_source,
};
use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Deserialize)]
struct Metadata {
    workspace_members: Vec<String>,
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    manifest_path: PathBuf,
    targets: Vec<Target>,
}

#[derive(Deserialize)]
struct Target {
    kind: Vec<String>,
    src_path: PathBuf,
}

struct Source {
    owner: String,
    analysis: RustSourceAnalysis,
    links: Vec<(PathBuf, RustSourceUse)>,
}

/// Resolves every workspace Rust file, including untracked source and non-host cfg branches.
pub(super) fn collect(workspace: &Path) -> Result<BTreeMap<PathBuf, ModuleSize>> {
    let workspace = CanonicalPathRoot::new(workspace)?;
    let metadata = Command::new("cargo")
        .current_dir(workspace.as_path())
        .args([
            "metadata",
            "--format-version",
            "1",
            "--no-deps",
            "--locked",
            "--offline",
        ])
        .output()?;
    if !metadata.status.success() {
        return Err(format!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&metadata.stderr)
        )
        .into());
    }
    let metadata: Metadata = serde_json::from_slice(&metadata.stdout)?;
    let packages: Vec<_> = metadata
        .packages
        .into_iter()
        .filter(|package| metadata.workspace_members.contains(&package.id))
        .collect();
    let mut owners = Vec::new();
    let mut roots = BTreeMap::new();
    for package in packages {
        let directory = package
            .manifest_path
            .parent()
            .ok_or("Cargo manifest has no parent")?;
        owners.push((
            workspace.resolve_existing_absolute(directory)?,
            package.name,
        ));
        for target in package.targets {
            let usage = if target
                .kind
                .iter()
                .any(|kind| kind == "test" || kind == "bench")
            {
                RustSourceUse::TestOnly
            } else {
                RustSourceUse::Production
            };
            let path = workspace.resolve_existing_absolute(&target.src_path)?;
            roots
                .entry(path)
                .and_modify(|current| {
                    if usage == RustSourceUse::Production {
                        *current = usage;
                    }
                })
                .or_insert(usage);
        }
    }
    // Deeper package paths win when an application contains another workspace package.
    owners.sort_by_key(|(directory, _)| std::cmp::Reverse(directory.components().count()));
    let files = Command::new("git")
        .current_dir(workspace.as_path())
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "-z",
            "--",
            "*.rs",
        ])
        .output()?;
    if !files.status.success() {
        return Err(format!(
            "git ls-files failed: {}",
            String::from_utf8_lossy(&files.stderr)
        )
        .into());
    }
    let mut sources = BTreeMap::new();
    for relative in files
        .stdout
        .split(|byte| *byte == 0)
        .filter(|name| !name.is_empty())
    {
        let filename = workspace.as_path().join(std::str::from_utf8(relative)?);
        // Staged deletions and unstaged removals are absent from this checkout, not stale exceptions.
        if !filename.is_file() {
            continue;
        }
        let filename = workspace.resolve_existing_absolute(&filename)?;
        let Some((_, owner)) = owners
            .iter()
            .find(|(directory, _)| filename.starts_with(directory))
        else {
            continue;
        };
        let kind = if roots.contains_key(&filename) {
            RustSourceKind::CrateRoot
        } else {
            RustSourceKind::Module
        };
        let analysis = analyze_rust_source(&fs::read_to_string(&filename)?, &filename, kind)
            .map_err(|error| format!("{}: {error}", filename.display()))?;
        let mut links = Vec::new();
        for module in &analysis.modules {
            for candidate in &module.candidates {
                if candidate.is_file() {
                    links.push((
                        workspace.resolve_existing_absolute(candidate)?,
                        module.usage,
                    ));
                }
            }
        }
        sources.insert(
            filename,
            Source {
                owner: owner.clone(),
                analysis,
                links,
            },
        );
    }
    for root in roots.keys() {
        if !sources.contains_key(root) {
            return Err(format!(
                "{}: Cargo root is missing from repository source accounting",
                root.display()
            )
            .into());
        }
    }
    account(workspace.as_path(), sources, roots)
}

/// Follows both production and test references; production always wins for shared source files.
fn account(
    workspace: &Path,
    sources: BTreeMap<PathBuf, Source>,
    roots: BTreeMap<PathBuf, RustSourceUse>,
) -> Result<BTreeMap<PathBuf, ModuleSize>> {
    let referenced: BTreeSet<_> = sources
        .values()
        .flat_map(|source| source.links.iter().map(|(path, _)| path.clone()))
        .collect();
    let root_paths: BTreeSet<_> = roots.keys().cloned().collect();
    let mut pending: VecDeque<_> = roots.into_iter().collect();
    // Unreferenced files are conservatively production. Their test-only children still follow cfg.
    pending.extend(
        sources
            .keys()
            .filter(|path| !referenced.contains(*path) && !root_paths.contains(*path))
            .map(|path| (path.clone(), RustSourceUse::Production)),
    );
    let mut reached = BTreeMap::new();
    while let Some((path, usage)) = pending.pop_front() {
        if reached
            .get(&path)
            .is_some_and(|previous| *previous == RustSourceUse::Production || *previous == usage)
        {
            continue;
        }
        reached.insert(path.clone(), usage);
        let source = sources.get(&path).ok_or_else(|| {
            format!(
                "{}: referenced Rust module is outside source accounting",
                path.display()
            )
        })?;
        for (child, child_usage) in &source.links {
            let child_usage = if usage == RustSourceUse::Production {
                *child_usage
            } else {
                RustSourceUse::TestOnly
            };
            pending.push_back((child.clone(), child_usage));
        }
    }
    sources
        .into_iter()
        .map(|(path, source)| {
            // Unreachable cycles are counted, never used as a reason to exclude source.
            let test_only = reached.get(&path) == Some(&RustSourceUse::TestOnly);
            let relative = path.strip_prefix(workspace)?.to_path_buf();
            Ok((
                relative,
                ModuleSize {
                    owner: source.owner,
                    production_lines: if test_only {
                        0
                    } else {
                        source.analysis.production_lines
                    },
                    test_only,
                },
            ))
        })
        .collect()
}

#[cfg(test)]
mod tests;
