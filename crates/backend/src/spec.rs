use crate::{BackendError, ErrorClassification};
use ora_application::{
    ListProjectSpecSourceOverridesHandler, ProjectRepository, TaskRepository,
    UpdateProjectSpecSourcesHandler, UuidProjectSpecSourceOverrideIdGenerator,
};
use ora_contracts::{
    EmptyErrorParams, GetSpecCatalogRequest, PublicError, ReadSpecRequest, ReadSpecResponse,
    ResolveSpecSourceRequest, ResolveSpecSourceResponse, SpecCatalogResponse, SpecDocument,
    SpecSource, SpecSourceAvailability, SpecSourceOrigin, SpecSourceVisibility, SpecTarget,
    SpecWorkflow as ContractWorkflow, UpdateProjectSpecSourcesRequest,
    UpdateProjectSpecSourcesResponse,
};
use ora_db::{
    RepositoryPool, SqliteProjectRepository, SqliteProjectSpecSourceOverrideRepository,
    SqliteTaskRepository,
};
use ora_domain::{
    ProjectId, SpecSourceVisibility as DomainVisibility, SpecWorkflow as DomainWorkflow, TaskId,
};
use ora_fs::WorkspaceFileSystem;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::clock::SystemClock;

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
        let overrides = ListProjectSpecSourceOverridesHandler::new(
            SqliteProjectSpecSourceOverrideRepository::new(self.pool.clone()),
        )
        .handle(&context.project_id)
        .map_err(BackendError::from)?;

        let mut candidates = default_candidates();
        for file in &discovered.files {
            for (path, workflow) in infer_sources(&file.path) {
                insert_candidate(
                    &mut candidates,
                    SourceCandidate {
                        relative_path: path,
                        workflow,
                        origin: SpecSourceOrigin::Discovered,
                        visibility: SpecSourceVisibility::Enabled,
                        configured: false,
                    },
                );
            }
        }
        for source in overrides {
            let key = source_key(&source.relative_path);
            let origin = candidates
                .get(&key)
                .map_or(SpecSourceOrigin::Manual, |candidate| candidate.origin);
            candidates.insert(
                key,
                SourceCandidate {
                    relative_path: source.relative_path,
                    workflow: map_domain_workflow(source.workflow),
                    origin,
                    visibility: map_domain_visibility(source.visibility),
                    configured: true,
                },
            );
        }

        let mut sources = candidates
            .into_values()
            .map(|mut candidate| {
                let availability = match self.file_system.resolve_spec_source(
                    &context.root,
                    &context.root.join(&candidate.relative_path),
                ) {
                    Ok(relative_path) => {
                        // Preserve canonical on-disk spelling so later path ownership checks
                        // remain correct on both case-sensitive and case-insensitive filesystems.
                        candidate.relative_path = relative_path;
                        SpecSourceAvailability::Available
                    }
                    Err(_) => SpecSourceAvailability::Missing,
                };
                EffectiveSource {
                    source: SpecSource {
                        relative_path: candidate.relative_path,
                        workflow: candidate.workflow,
                        origin: candidate.origin,
                        visibility: candidate.visibility,
                        availability,
                    },
                    configured: candidate.configured,
                }
            })
            .collect::<Vec<_>>();
        sources.sort_by(|left, right| {
            left.source
                .relative_path
                .to_lowercase()
                .cmp(&right.source.relative_path.to_lowercase())
        });

        let enabled_paths = sources
            .iter()
            .filter(|source| {
                source.source.visibility == SpecSourceVisibility::Enabled
                    && source.source.availability == SpecSourceAvailability::Available
            })
            .map(|source| source.source.relative_path.clone())
            .collect::<Vec<_>>();
        let explicit = self
            .file_system
            .enumerate_spec_sources(&context.root, &enabled_paths)
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
                let owner = select_source(&sources, &file.path)?;
                Some(SpecDocument {
                    relative_path: file.path,
                    source_relative_path: owner.source.relative_path.clone(),
                    workflow: owner.source.workflow.clone(),
                    byte_size: u32::try_from(file.size_bytes).unwrap_or(u32::MAX),
                })
            })
            .collect();

        Ok(SpecCatalogResponse {
            sources: sources.into_iter().map(|source| source.source).collect(),
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

    /// Validates a platform picker result and infers its initial workflow classification.
    pub(crate) fn resolve_source(
        &self,
        request: ResolveSpecSourceRequest,
    ) -> Result<ResolveSpecSourceResponse, BackendError> {
        let context = self.resolve_target(&request.target)?;
        let relative_path = self
            .file_system
            .resolve_spec_source(&context.root, Path::new(&request.absolute_path))
            .map_err(spec_source_error)?;
        if relative_path.is_empty() {
            return Err(spec_source_workspace_root());
        }
        let workflow = infer_sources(&format!("{relative_path}/placeholder.md"))
            .into_iter()
            .last()
            .map_or_else(
                || ContractWorkflow::Custom {
                    name: "Custom".to_string(),
                },
                |(_, workflow)| workflow,
            );
        Ok(ResolveSpecSourceResponse {
            relative_path,
            workflow,
        })
    }

    /// Runs the application-owned atomic replacement handler.
    pub(crate) fn update_project_sources(
        &self,
        request: UpdateProjectSpecSourcesRequest,
    ) -> Result<UpdateProjectSpecSourcesResponse, BackendError> {
        let handler = UpdateProjectSpecSourcesHandler::new(
            SqliteProjectSpecSourceOverrideRepository::new(self.pool.clone()),
            SqliteProjectRepository::new(self.pool.clone()),
            UuidProjectSpecSourceOverrideIdGenerator,
            SystemClock,
        );
        handler.handle(request).map_err(BackendError::from)
    }

    /// Resolves a watch target to the same authoritative root used by catalog and read operations.
    pub(crate) fn watch_root(&self, target: &SpecTarget) -> Result<PathBuf, BackendError> {
        self.resolve_target(target).map(|context| context.root)
    }

    /// Resolves target ownership once so worktree and project-root semantics cannot diverge by operation.
    fn resolve_target(&self, target: &SpecTarget) -> Result<SpecContext, BackendError> {
        match target {
            SpecTarget::Project { project_id } => {
                let project_id = ProjectId::new(project_id);
                let project = SqliteProjectRepository::new(self.pool.clone())
                    .find_project(&project_id)
                    .map_err(|source| {
                        BackendError::internal("project repository operation failed", source)
                    })?
                    .ok_or_else(|| project_not_found(&project_id))?;
                let root = crate::task::absolute_project_root(
                    PathBuf::from(project.root_path),
                    &self.relative_path_base,
                )?;
                Ok(SpecContext {
                    project_id,
                    root,
                    _worktree_use: None,
                })
            }
            SpecTarget::Task { task_id } => {
                // Shared use lease: keeps the checkout on disk while spec files
                // resolved from it are being read; dropped with the context.
                let worktree_use = self.git_cleanup.shared_worktree_use(task_id);
                let task_id = TaskId::new(task_id);
                let task = SqliteTaskRepository::new(self.pool.clone())
                    .find_task(&task_id)
                    .map_err(|source| {
                        BackendError::internal("task repository operation failed", source)
                    })?
                    .ok_or_else(|| task_not_found(&task_id))?;
                let root =
                    crate::task::resolve_task_cwd(&self.pool, &task_id, &self.relative_path_base)?;
                Ok(SpecContext {
                    project_id: task.project_id,
                    root,
                    _worktree_use: Some(worktree_use),
                })
            }
        }
    }
}

struct SpecContext {
    project_id: ProjectId,
    root: PathBuf,
    /// Holds the task checkout on disk for the lifetime of this resolution.
    _worktree_use: Option<crate::git_cleanup::SharedLeaseGuard>,
}

struct SourceCandidate {
    relative_path: String,
    workflow: ContractWorkflow,
    origin: SpecSourceOrigin,
    visibility: SpecSourceVisibility,
    configured: bool,
}

struct EffectiveSource {
    source: SpecSource,
    configured: bool,
}

/// Builds Ora's built-in source candidates before discovery and project overrides are applied.
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
                origin: SpecSourceOrigin::Default,
                visibility: SpecSourceVisibility::Enabled,
                configured: false,
            },
        )
    })
    .collect()
}

/// Keeps explicit/default candidates ahead of inferred duplicates while still adding new discoveries.
fn insert_candidate(
    candidates: &mut BTreeMap<String, SourceCandidate>,
    candidate: SourceCandidate,
) {
    let key = source_key(&candidate.relative_path);
    match candidates.get_mut(&key) {
        Some(existing) if !existing.configured => {
            // Preserve the higher-confidence default classification while using the exact
            // on-disk spelling required by case-sensitive filesystems.
            existing.relative_path = candidate.relative_path;
        }
        Some(_) => {}
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

/// Chooses the deepest enabled available source; a configured tie wins over inference.
fn select_source<'a>(
    sources: &'a [EffectiveSource],
    file_path: &str,
) -> Option<&'a EffectiveSource> {
    sources
        .iter()
        .filter(|candidate| {
            candidate.source.visibility == SpecSourceVisibility::Enabled
                && candidate.source.availability == SpecSourceAvailability::Available
                && path_is_within(file_path, &candidate.source.relative_path)
        })
        .max_by_key(|candidate| {
            (
                candidate.source.relative_path.split('/').count(),
                usize::from(candidate.configured),
            )
        })
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

/// Converts the persistence workflow into its transport representation.
fn map_domain_workflow(workflow: DomainWorkflow) -> ContractWorkflow {
    match workflow {
        DomainWorkflow::OpenSpec => ContractWorkflow::OpenSpec,
        DomainWorkflow::Superpowers => ContractWorkflow::Superpowers,
        DomainWorkflow::Custom { name } => ContractWorkflow::Custom { name },
    }
}

/// Converts persisted visibility into the public source state.
fn map_domain_visibility(visibility: DomainVisibility) -> SpecSourceVisibility {
    match visibility {
        DomainVisibility::Enabled => SpecSourceVisibility::Enabled,
        DomainVisibility::Disabled => SpecSourceVisibility::Disabled,
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

/// Maps platform-picker failures to stable public errors without exposing local paths.
fn spec_source_error(source: ora_fs::WorkspaceFileSystemError) -> BackendError {
    let (classification, public_error, context) = match source {
        ora_fs::WorkspaceFileSystemError::PathNotRelative { .. } => (
            ErrorClassification::InvalidRequest,
            PublicError::FileSystemPathNotAbsolute(EmptyErrorParams {}),
            "specification source path must be absolute",
        ),
        ora_fs::WorkspaceFileSystemError::PathNotFound { .. } => (
            ErrorClassification::NotFound,
            PublicError::FileSystemPathNotFound(EmptyErrorParams {}),
            "specification source path was not found",
        ),
        ora_fs::WorkspaceFileSystemError::NotDirectory { .. } => (
            ErrorClassification::InvalidRequest,
            PublicError::FileSystemPathNotDirectory(EmptyErrorParams {}),
            "specification source path is not a directory",
        ),
        ora_fs::WorkspaceFileSystemError::PathOutsideWorkspace { .. } => (
            ErrorClassification::InvalidRequest,
            PublicError::SpecSourceOutsideWorkspace(EmptyErrorParams {}),
            "specification source path is outside the workspace",
        ),
        ora_fs::WorkspaceFileSystemError::Io { ref source, .. }
            if source.kind() == std::io::ErrorKind::PermissionDenied =>
        {
            (
                ErrorClassification::InvalidRequest,
                PublicError::FileSystemPathPermissionDenied(EmptyErrorParams {}),
                "specification source path is not readable",
            )
        }
        ora_fs::WorkspaceFileSystemError::WorkspaceUnavailable { .. }
        | ora_fs::WorkspaceFileSystemError::Io { .. }
        | ora_fs::WorkspaceFileSystemError::NotFile { .. }
        | ora_fs::WorkspaceFileSystemError::FileTooLarge { .. }
        | ora_fs::WorkspaceFileSystemError::BinaryFile { .. }
        | ora_fs::WorkspaceFileSystemError::InvalidUtf8 { .. }
        | ora_fs::WorkspaceFileSystemError::SearchToolUnavailable { .. }
        | ora_fs::WorkspaceFileSystemError::SearchTimedOut
        | ora_fs::WorkspaceFileSystemError::SearchOutputTooLarge { .. }
        | ora_fs::WorkspaceFileSystemError::SearchFailed { .. }
        | ora_fs::WorkspaceFileSystemError::InvalidSearchOutput { .. }
        | ora_fs::WorkspaceFileSystemError::WatchFailed { .. } => (
            ErrorClassification::Internal,
            PublicError::InternalError(EmptyErrorParams {}),
            "specification source resolution failed",
        ),
    };
    BackendError::with_source(classification, public_error, context, source)
}

/// Builds the public error when the picker selects the workspace root instead of a subdirectory.
fn spec_source_workspace_root() -> BackendError {
    BackendError::new(
        ErrorClassification::InvalidRequest,
        PublicError::SpecSourceWorkspaceRoot(EmptyErrorParams {}),
        "specification source cannot be the workspace root",
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
    use super::{
        EffectiveSource, infer_sources, path_is_within, select_source, spec_source_error,
        spec_source_workspace_root,
    };
    use crate::ErrorClassification;
    use ora_contracts::{
        EmptyErrorParams, PublicError, SpecSource, SpecSourceAvailability, SpecSourceOrigin,
        SpecSourceVisibility, SpecWorkflow,
    };
    use ora_fs::WorkspaceFileSystemError;
    use pretty_assertions::assert_eq;
    use std::path::PathBuf;

    /// Verifies picker containment failures surface dedicated public error codes.
    #[test]
    fn maps_spec_source_filesystem_failures_to_public_errors() {
        assert_eq!(
            spec_source_error(WorkspaceFileSystemError::PathOutsideWorkspace {
                path: PathBuf::from("C:/outside"),
            })
            .public_error(),
            &PublicError::SpecSourceOutsideWorkspace(EmptyErrorParams {})
        );
        assert_eq!(
            spec_source_error(WorkspaceFileSystemError::NotDirectory {
                path: PathBuf::from("C:/repo/file.txt"),
            })
            .public_error(),
            &PublicError::FileSystemPathNotDirectory(EmptyErrorParams {})
        );
        assert_eq!(
            spec_source_workspace_root().public_error(),
            &PublicError::SpecSourceWorkspaceRoot(EmptyErrorParams {})
        );
        assert_eq!(
            spec_source_workspace_root().classification(),
            ErrorClassification::InvalidRequest
        );
    }

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
        let sources = [source("docs/specs", false), source("docs/specs/api", true)];
        let selected = select_source(&sources, "docs/specs/api/auth.md").unwrap();

        assert_eq!(selected.source.relative_path, "docs/specs/api");
        assert!(path_is_within("docs/specs/a.md", "docs/specs"));
        assert!(!path_is_within("docs/specs-old/a.md", "docs/specs"));
    }

    /// Builds one enabled source fixture for ownership selection tests.
    fn source(relative_path: &str, configured: bool) -> EffectiveSource {
        EffectiveSource {
            source: SpecSource {
                relative_path: relative_path.to_string(),
                workflow: SpecWorkflow::Custom {
                    name: "Custom".to_string(),
                },
                origin: SpecSourceOrigin::Discovered,
                visibility: SpecSourceVisibility::Enabled,
                availability: SpecSourceAvailability::Available,
            },
            configured,
        }
    }
}
