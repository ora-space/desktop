use crate::{BackendError, ErrorClassification};
use ora_application::TaskRepository;
use ora_contracts::{
    EmptyErrorParams, GetSpecCatalogRequest, PublicError, ReadSpecRequest, ReadSpecResponse,
    SpecCatalogResponse, SpecDocument, SpecTarget, SpecWorkflow as ContractWorkflow,
};
use ora_db::{RepositoryPool, SqliteTaskRepository, SqliteWorkspaceRepository};
use ora_domain::{ProjectId, TaskId};
use ora_fs::WorkspaceFileSystem;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Composes project configuration, target resolution, and bounded filesystem discovery.
pub(crate) struct SpecApi {
    pool: RepositoryPool,
    file_system: WorkspaceFileSystem,
    git_cleanup: crate::git_cleanup::GitCleanupHandle,
    relative_path_base: PathBuf,
}

impl SpecApi {
    /// Builds the shared Spec API with Ora's bundled ripgrep path.
    pub(crate) fn new(
        pool: RepositoryPool,
        ripgrep_path: PathBuf,
        git_cleanup: crate::git_cleanup::GitCleanupHandle,
        relative_path_base: PathBuf,
    ) -> Self {
        Self {
            pool,
            file_system: WorkspaceFileSystem::system(ripgrep_path),
            git_cleanup,
            relative_path_base,
        }
    }

    /// Builds the effective source catalog and assigns every Markdown file to its most specific source.
    pub(crate) async fn catalog(
        &self,
        request: GetSpecCatalogRequest,
    ) -> Result<SpecCatalogResponse, BackendError> {
        let context = self.resolve_target(&request.target)?;
        let discovered = self
            .file_system
            .discover_spec_markdown(&context.root)
            .await
            .map_err(spec_filesystem_error)?;
        let mut candidates = default_candidates();
        for file in &discovered.files {
            for (path, workflow) in infer_sources(&file.path) {
                insert_candidate(
                    &mut candidates,
                    SourceCandidate {
                        relative_path: path,
                        workflow,
                    },
                );
            }
        }
        let source_paths = candidates
            .values()
            .map(|source| source.relative_path.clone())
            .collect::<Vec<_>>();
        let explicit = self
            .file_system
            .enumerate_spec_sources(&context.root, &source_paths)
            .await
            .map_err(spec_filesystem_error)?;
        let mut indexed_files = discovered
            .files
            .into_iter()
            .map(|file| (file.path.clone(), file))
            .collect::<BTreeMap<_, _>>();
        indexed_files.extend(
            explicit
                .files
                .into_iter()
                .map(|file| (file.path.clone(), file)),
        );

        let documents = indexed_files
            .into_values()
            .filter_map(|file| {
                let owner = select_source(&candidates, &file.path)?;
                Some(SpecDocument {
                    relative_path: file.path,
                    source_relative_path: owner.relative_path.clone(),
                    workflow: owner.workflow.clone(),
                    byte_size: u32::try_from(file.size_bytes).unwrap_or(u32::MAX),
                })
            })
            .collect();

        Ok(SpecCatalogResponse {
            documents,
            truncated: discovered.truncated || explicit.truncated,
        })
    }

    /// Reads one document only after revalidating membership in the current effective catalog.
    pub(crate) async fn read(
        &self,
        request: ReadSpecRequest,
    ) -> Result<ReadSpecResponse, BackendError> {
        let catalog = self
            .catalog(GetSpecCatalogRequest {
                target: request.target.clone(),
            })
            .await?;
        if !catalog
            .documents
            .iter()
            .any(|document| document.relative_path == request.relative_path)
        {
            return Err(spec_document_not_found());
        }
        let context = self.resolve_target(&request.target)?;
        let file = self
            .file_system
            .read_spec_file(&context.root, Path::new(&request.relative_path))
            .map_err(spec_filesystem_error)?;
        Ok(ReadSpecResponse {
            relative_path: file.path,
            content: file.content,
            byte_size: u32::try_from(file.size_bytes).unwrap_or(u32::MAX),
        })
    }

    /// Resolves a watch target to the same authoritative root used by catalog and read operations.
    pub(crate) fn watch_root(&self, target: &SpecTarget) -> Result<PathBuf, BackendError> {
        self.resolve_target(target).map(|context| context.root)
    }

    /// Resolves target ownership once so worktree and main-Workspace semantics cannot diverge by operation.
    fn resolve_target(&self, target: &SpecTarget) -> Result<SpecContext, BackendError> {
        match target {
            SpecTarget::Project { project_id } => {
                let project_id = ProjectId::new(project_id);
                let workspace = SqliteWorkspaceRepository::new(self.pool.clone())
                    .find_main_workspace(&project_id)
                    .map_err(|source| {
                        BackendError::internal("workspace repository operation failed", source)
                    })?
                    .ok_or_else(|| project_not_found(&project_id))?;
                if !workspace.is_admissible() {
                    return Err(crate::task::workspace_unavailable());
                }
                let ora_domain::WorkspaceLocation::LocalFilesystem { path } = workspace.location
                else {
                    return Err(crate::task::workspace_unavailable());
                };
                let root = crate::task::absolute_project_root(
                    PathBuf::from(path),
                    &self.relative_path_base,
                )?;
                Ok(SpecContext {
                    root,
                    _worktree_use: None,
                })
            }
            SpecTarget::Task { task_id } => {
                let task_id = TaskId::new(task_id);
                let task = SqliteTaskRepository::new(self.pool.clone())
                    .find_task(&task_id)
                    .map_err(|source| {
                        BackendError::internal("task repository operation failed", source)
                    })?
                    .ok_or_else(|| task_not_found(&task_id))?;
                // Shared use lease: keeps the Workspace checkout on disk while
                // spec files resolved from it are being read; dropped with the context.
                let worktree_use = self
                    .git_cleanup
                    .shared_worktree_use(task.workspace_id.as_ref());
                let root =
                    crate::task::resolve_task_cwd(&self.pool, &task_id, &self.relative_path_base)?;
                Ok(SpecContext {
                    root,
                    _worktree_use: Some(worktree_use),
                })
            }
        }
    }
}

struct SpecContext {
    root: PathBuf,
    /// Holds the task checkout on disk for the lifetime of this resolution.
    _worktree_use: Option<crate::git_cleanup::SharedLeaseGuard>,
}

struct SourceCandidate {
    relative_path: String,
    workflow: ContractWorkflow,
}

/// Builds Ora's built-in source candidates before filesystem discovery is applied.
fn default_candidates() -> BTreeMap<String, SourceCandidate> {
    [
        ("openspec/specs", ContractWorkflow::OpenSpec),
        ("openspec/changes", ContractWorkflow::OpenSpec),
        ("docs/superpowers/specs", ContractWorkflow::Superpowers),
        ("docs/superpowers/plans", ContractWorkflow::Superpowers),
        ("docs/plans", ContractWorkflow::Superpowers),
        (
            "specs",
            ContractWorkflow::Custom {
                name: "Custom".to_string(),
            },
        ),
        (
            "docs/specs",
            ContractWorkflow::Custom {
                name: "Custom".to_string(),
            },
        ),
    ]
    .into_iter()
    .map(|(relative_path, workflow)| {
        (
            source_key(relative_path),
            SourceCandidate {
                relative_path: relative_path.to_string(),
                workflow,
            },
        )
    })
    .collect()
}

/// Keeps built-in classifications ahead of inferred duplicates while preserving on-disk spelling.
fn insert_candidate(
    candidates: &mut BTreeMap<String, SourceCandidate>,
    candidate: SourceCandidate,
) {
    let key = source_key(&candidate.relative_path);
    match candidates.get_mut(&key) {
        Some(existing) => {
            // Preserve the higher-confidence default classification while using the exact
            // on-disk spelling required by case-sensitive filesystems.
            existing.relative_path = candidate.relative_path;
        }
        None => {
            candidates.insert(key, candidate);
        }
    }
}

/// Infers every controlled spec directory and workflow-owned plan/change directory in a file path.
fn infer_sources(file_path: &str) -> Vec<(String, ContractWorkflow)> {
    let segments = file_path.split('/').collect::<Vec<_>>();
    let lower = segments
        .iter()
        .map(|segment| segment.to_lowercase())
        .collect::<Vec<_>>();
    let mut inferred = Vec::new();
    for (index, segment) in lower.iter().enumerate().take(lower.len().saturating_sub(1)) {
        let openspec = lower[..index].iter().rposition(|owner| owner == "openspec");
        let superpowers = lower[..index]
            .iter()
            .rposition(|owner| owner == "superpowers");
        let is_spec = segment == "spec" || segment == "specs";
        let is_openspec_change = segment == "changes" && openspec.is_some();
        let is_superpowers_plan = segment == "plans" && superpowers.is_some();
        if is_spec || is_openspec_change || is_superpowers_plan {
            let workflow = if is_openspec_change {
                ContractWorkflow::OpenSpec
            } else if is_superpowers_plan {
                ContractWorkflow::Superpowers
            } else {
                match (openspec, superpowers) {
                    (Some(open_index), Some(super_index)) if open_index > super_index => {
                        ContractWorkflow::OpenSpec
                    }
                    (Some(_), Some(_)) | (None, Some(_)) => ContractWorkflow::Superpowers,
                    (Some(_), None) => ContractWorkflow::OpenSpec,
                    (None, None) => ContractWorkflow::Custom {
                        name: "Custom".to_string(),
                    },
                }
            };
            inferred.push((segments[..=index].join("/"), workflow));
        }
    }
    inferred
}

/// Chooses the deepest inferred or built-in source that owns the file.
fn select_source<'a>(
    sources: &'a BTreeMap<String, SourceCandidate>,
    file_path: &str,
) -> Option<&'a SourceCandidate> {
    sources
        .values()
        .filter(|candidate| path_is_within(file_path, &candidate.relative_path))
        .max_by_key(|candidate| candidate.relative_path.split('/').count())
}

/// Tests source ownership on normalized slash-separated path segment boundaries.
fn path_is_within(file_path: &str, source_path: &str) -> bool {
    let file_segments = file_path.split('/').collect::<Vec<_>>();
    let source_segments = source_path.split('/').collect::<Vec<_>>();
    file_segments.len() >= source_segments.len()
        && file_segments
            .iter()
            .zip(source_segments)
            .all(|(file, source)| path_segment_eq(file, source))
}

/// Uses the host filesystem's case semantics when identifying duplicate source paths.
fn source_key(relative_path: &str) -> String {
    if cfg!(windows) {
        relative_path.to_lowercase()
    } else {
        relative_path.to_string()
    }
}

/// Compares one normalized path segment according to the host filesystem's case semantics.
fn path_segment_eq(left: &str, right: &str) -> bool {
    if cfg!(windows) {
        left.eq_ignore_ascii_case(right)
    } else {
        left == right
    }
}

/// Builds the stable not-found response for a missing project target.
fn project_not_found(project_id: &ProjectId) -> BackendError {
    BackendError::new(
        ErrorClassification::NotFound,
        PublicError::ProjectNotFound(EmptyErrorParams {}),
        format!("project not found: {project_id}"),
    )
}

/// Builds the stable not-found response for a missing task target.
fn task_not_found(task_id: &TaskId) -> BackendError {
    BackendError::new(
        ErrorClassification::NotFound,
        PublicError::TaskNotFound(EmptyErrorParams {}),
        format!("task not found: {task_id}"),
    )
}

/// Keeps discovery and read failures private because their details may expose local paths.
fn spec_filesystem_error(source: ora_fs::WorkspaceFileSystemError) -> BackendError {
    BackendError::internal("specification filesystem operation failed", source)
}

/// Builds the stable not-found response for a document outside the effective catalog.
fn spec_document_not_found() -> BackendError {
    BackendError::new(
        ErrorClassification::NotFound,
        PublicError::SpecDocumentNotFound(EmptyErrorParams {}),
        "specification document is not in the current catalog",
    )
}

#[cfg(test)]
mod tests {
    use super::{SourceCandidate, infer_sources, path_is_within, select_source, source_key};
    use ora_contracts::SpecWorkflow;
    use pretty_assertions::assert_eq;
    use std::collections::BTreeMap;

    /// Verifies controlled discovery recognizes workflow-owned and generic spec directories.
    #[test]
    fn infers_supported_source_directories() {
        assert_eq!(
            infer_sources("openspec/changes/add-search/proposal.md"),
            vec![("openspec/changes".to_string(), SpecWorkflow::OpenSpec)]
        );
        assert_eq!(
            infer_sources("tools/superpowers/plans/release.MDX"),
            vec![(
                "tools/superpowers/plans".to_string(),
                SpecWorkflow::Superpowers
            )]
        );
        assert_eq!(
            infer_sources("architecture/spec/api/design.md"),
            vec![(
                "architecture/spec".to_string(),
                SpecWorkflow::Custom {
                    name: "Custom".to_string()
                },
            )]
        );
        assert_eq!(
            infer_sources("docs/specs/api/specs/auth.md"),
            vec![
                (
                    "docs/specs".to_string(),
                    SpecWorkflow::Custom {
                        name: "Custom".to_string()
                    },
                ),
                (
                    "docs/specs/api/specs".to_string(),
                    SpecWorkflow::Custom {
                        name: "Custom".to_string()
                    },
                ),
            ]
        );
        assert_eq!(
            infer_sources("openspec/vendor/superpowers/specs/release.md"),
            vec![(
                "openspec/vendor/superpowers/specs".to_string(),
                SpecWorkflow::Superpowers,
            )]
        );
        assert_eq!(infer_sources("docs/notes/readme.md"), vec![]);
    }

    /// Verifies overlapping enabled sources assign a document to the deepest directory.
    #[test]
    fn assigns_documents_to_the_most_specific_source() {
        let sources = BTreeMap::from([source("docs/specs"), source("docs/specs/api")]);
        let selected = select_source(&sources, "docs/specs/api/auth.md").unwrap();

        assert_eq!(selected.relative_path, "docs/specs/api");
        assert!(path_is_within("docs/specs/a.md", "docs/specs"));
        assert!(!path_is_within("docs/specs-old/a.md", "docs/specs"));
    }

    /// Builds one enabled source fixture for ownership selection tests.
    fn source(relative_path: &str) -> (String, SourceCandidate) {
        (
            source_key(relative_path),
            SourceCandidate {
                relative_path: relative_path.to_string(),
                workflow: SpecWorkflow::Custom {
                    name: "Custom".to_string(),
                },
            },
        )
    }
}
