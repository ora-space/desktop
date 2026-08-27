//! Derives the surface registrations that no consumer declaration could have created.
//!
//! A consumer publishes its Effect surfaces when its plugin process starts, and that publication
//! can only reach the Workspaces that exist at that moment. A Workspace created afterwards already
//! owns a complete Desired set — the `workspaces` insert trigger seeds it from every active source
//! — but owns no surface to project it onto, so nothing ever enters the reconcile queue for it and
//! no amount of waiting materializes anything.
//!
//! Making the surface set a convergence result rather than the side effect of one process event is
//! what removes the ordering dependency. The worker re-derives it every pass, so a Workspace
//! created at any moment relative to a plugin's start reaches the same state without a restart.

use crate::error::BackendError;
use ora_db::SqliteEffectRepository;
use ora_domain::{Workspace, WorkspaceLocation};
use ora_effect::{FilesystemSkillSurface, SurfaceDescriptorSet};
use std::path::Path;

/// Registers the current declarations into every local Workspace that owns no active surface.
///
/// Returns how many Workspaces were registered, which is zero in the steady state.
pub(crate) fn converge_workspace_surfaces(
    repository: &SqliteEffectRepository,
    workspaces: &[Workspace],
    declarations: &[FilesystemSkillSurface],
    now: i64,
) -> Result<usize, BackendError> {
    if declarations.is_empty() {
        // No consumer is asking for anything, so an unregistered Workspace owes no surface. This
        // is also the state of a process whose plugins have not started yet.
        return Ok(0);
    }
    let registered = repository
        .list_workspaces_with_active_surfaces()
        .map_err(|error| {
            BackendError::internal("failed to list registered Effect Workspaces", error)
        })?;
    let mut converged = 0;
    for workspace in workspaces {
        if registered.contains(&workspace.id) {
            continue;
        }
        // Filesystem-only, matching declaration-time registration: a remote Workspace needs a
        // provider-owned adapter rather than an opaque locator treated as a host path.
        let WorkspaceLocation::LocalFilesystem { path } = &workspace.location else {
            continue;
        };
        let merged = SurfaceDescriptorSet::merge(&workspace.id, declarations.to_vec())
            .map_err(|error| BackendError::internal("invalid Agent Effect surface", error))?;
        repository
            .replace_surfaces(&workspace.id, Path::new(path), &merged, now)
            .map_err(|error| {
                BackendError::internal("failed to register Effect surfaces for a Workspace", error)
            })?;
        converged += 1;
    }
    Ok(converged)
}

#[cfg(test)]
mod tests {
    use super::converge_workspace_surfaces;
    use crate::project::ProjectApi;
    use ora_contracts::CreateProjectRequest;
    use ora_db::{
        DatabaseBootstrapper, DatabaseLocation, RepositoryPool, SqliteEffectRepository,
        SqliteWorkspaceRepository, default_migration_catalog,
    };
    use ora_domain::Workspace;
    use ora_effect::{
        ConsumerCoordination, ConsumerId, FilesystemSkillSurface, MaterializationFormat,
        SurfacePath,
    };
    use pretty_assertions::assert_eq;
    use std::path::Path;
    use tempfile::TempDir;

    const NOW: i64 = 1_800_000_000_000;

    /// Builds a pool holding one Workspace created through the real project path.
    fn fixture(data_root: &Path, workspace_root: &Path) -> RepositoryPool {
        let pool = DatabaseBootstrapper::system()
            .bootstrap_repository_pool(
                &DatabaseLocation::path(data_root.join("ora.sqlite3")),
                &default_migration_catalog().unwrap(),
            )
            .unwrap();
        std::fs::create_dir_all(workspace_root).unwrap();
        ProjectApi::new(
            pool.clone(),
            data_root.join("sessions"),
            crate::clock::SystemClock,
            crate::effect_worker::EffectWorkerHandle::unwatched(),
        )
        .create(CreateProjectRequest {
            name: "Demo".to_string(),
            main_workspace_path: workspace_root.to_string_lossy().into_owned(),
        })
        .unwrap();
        pool
    }

    fn workspaces(pool: &RepositoryPool) -> Vec<Workspace> {
        SqliteWorkspaceRepository::new(pool.clone())
            .list_all_workspaces()
            .unwrap()
    }

    /// One Agent-consumed surface, the same shape a plugin publishes at start.
    fn declarations() -> Vec<FilesystemSkillSurface> {
        vec![FilesystemSkillSurface {
            workspace_relative_path: SurfacePath::parse(".opencode/skills").unwrap(),
            materialization_format: MaterializationFormat::skill_directory_v1(),
            consumer: ConsumerId::new("official/ora-space.opencode"),
            coordination: ConsumerCoordination::WaitForIdleAndRestart,
        }]
    }

    #[test]
    fn an_unregistered_workspace_receives_the_current_declarations() {
        let temp = TempDir::new().unwrap();
        let pool = fixture(temp.path(), &temp.path().join("workspace"));
        let repository = SqliteEffectRepository::new(pool.clone());
        let workspaces = workspaces(&pool);
        assert_eq!(
            repository.list_workspaces_with_active_surfaces().unwrap(),
            Default::default()
        );

        let converged =
            converge_workspace_surfaces(&repository, &workspaces, &declarations(), NOW).unwrap();

        assert_eq!(converged, 1);
        assert_eq!(
            repository.list_workspaces_with_active_surfaces().unwrap(),
            workspaces.iter().map(|w| w.id.clone()).collect()
        );
    }

    #[test]
    fn an_already_registered_workspace_is_left_untouched() {
        let temp = TempDir::new().unwrap();
        let pool = fixture(temp.path(), &temp.path().join("workspace"));
        let repository = SqliteEffectRepository::new(pool.clone());
        let workspaces = workspaces(&pool);
        converge_workspace_surfaces(&repository, &workspaces, &declarations(), NOW).unwrap();

        // The steady state must be a no-op, or every pass would rewrite surfaces and re-arm
        // reconcile requests for Workspaces that are already current.
        let converged =
            converge_workspace_surfaces(&repository, &workspaces, &declarations(), NOW + 10)
                .unwrap();

        assert_eq!(converged, 0);
    }

    #[test]
    fn no_declarations_registers_nothing() {
        let temp = TempDir::new().unwrap();
        let pool = fixture(temp.path(), &temp.path().join("workspace"));
        let repository = SqliteEffectRepository::new(pool.clone());

        let converged =
            converge_workspace_surfaces(&repository, &workspaces(&pool), &[], NOW).unwrap();

        assert_eq!(converged, 0);
        assert_eq!(
            repository.list_workspaces_with_active_surfaces().unwrap(),
            Default::default()
        );
    }
}
