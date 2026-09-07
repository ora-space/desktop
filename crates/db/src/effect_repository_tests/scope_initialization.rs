use super::{fixture, package_fingerprint};
use crate::{
    PluginSkillProjection, SqliteProjectRepository, SqliteSkillRepository,
    SqliteTaskWorkspaceRepository, test_clock::TestClock,
};
use ora_application::{ProjectRepository, TaskWorkspaceCommit};
use ora_domain::{
    AuditFields, PluginId, Project, ProjectId, Task, TaskId, WorkspaceId, WorkspaceKind,
    WorkspaceLocation, Worktree, WorktreeActivity, WorktreeBaseline, WorktreeProvisioningLeaseId,
};
use ora_effect::Digest;
use pretty_assertions::assert_eq;
use rusqlite::params;

/// Main and task Workspaces must roll back with failed Effect seeding and use their own write time.
/// specs/test-cases/desktop/core/effect/time.md#workspace-and-effect-initialization-commit-atomically
#[test]
fn workspace_and_effect_initialization_commit_atomically() -> Result<(), Box<dyn std::error::Error>>
{
    for kind in [WorkspaceKind::Main, WorkspaceKind::Isolated] {
        let (directory, pool, _workspace) = fixture();
        let clock = TestClock::new(300);
        let package = directory.path().join("package");
        std::fs::create_dir_all(&package)?;
        std::fs::write(package.join("SKILL.md"), b"manifest")?;
        SqliteSkillRepository::with_clock(pool.clone(), clock.clone()).replace_plugin_skills(
            &PluginId::new("official", "review")?,
            "1",
            &[PluginSkillProjection {
                name: "review".to_string(),
                description: "Reviews changes".to_string(),
                package_fingerprint: package_fingerprint(&package),
                package_root: package,
                skill_md_digest: Digest::sha256(b"manifest"),
            }],
            /*updated_at*/ 10,
        )?;
        pool.with_connection(|connection| {
            connection.execute("INSERT INTO worktree_provisioning_leases
                (id, project_id, workspace_id, repository_root, checkout_root, branch_name, lease_expires_at, created_at, updated_at)
                VALUES ('lease', 'project-1', 'workspace-2', ?1, ?1, 'branch', 1000, 10, 10)", params![directory.path().to_string_lossy()])?;
            connection.execute_batch("CREATE TRIGGER reject_test_seed BEFORE INSERT ON effect_desired_effects
                BEGIN SELECT RAISE(ABORT, 'test seed failure'); END;")?;
            Ok(())
        })?;
        let create = || -> Result<(), Box<dyn std::error::Error>> {
            let audit = AuditFields::new(10, 10, /*is_deleted*/ false);
            match kind {
                WorkspaceKind::Main => {
                    SqliteProjectRepository::with_clock(pool.clone(), clock.clone())
                        .create_project(
                            Project::new(ProjectId::new("project-2"), "New project", audit),
                            WorkspaceLocation::local_filesystem(directory.path().to_string_lossy()),
                        )?;
                }
                WorkspaceKind::Isolated => {
                    let workspace = WorkspaceId::new("workspace-2");
                    SqliteTaskWorkspaceRepository::with_clock(pool.clone(), clock.clone())
                        .commit_worktree_task(
                            &Task::new(
                                TaskId::new("task"),
                                ProjectId::new("project-1"),
                                workspace.clone(),
                                "Task",
                                audit.clone(),
                            ),
                            &Worktree::new(
                                workspace,
                                Some("branch".to_string()),
                                WorktreeBaseline::recorded("baseline")?,
                                WorktreeActivity::Inactive,
                                audit,
                            ),
                            &WorktreeProvisioningLeaseId::new("lease"),
                        )?;
                }
            }
            Ok(())
        };
        assert!(create().is_err());
        let after_failure = pool.with_connection(|connection| connection.query_row(
            "SELECT (SELECT count(*) FROM projects), (SELECT count(*) FROM workspaces),
                    (SELECT count(*) FROM effect_scopes), (SELECT count(*) FROM effect_desired_effects),
                    (SELECT count(*) FROM tasks), (SELECT count(*) FROM worktree_provisioning_leases)",
            [], |row| (0..6).map(|index| row.get::<_, i64>(index)).collect::<Result<Vec<_>, _>>()).map_err(Into::into))?;
        assert_eq!(after_failure, vec![1, 1, 1, 1, 0, 1]);
        pool.with_connection(|connection| {
            connection.execute_batch("DROP TRIGGER reject_test_seed")?;
            Ok(())
        })?;
        create()?;
        let seeded = pool.with_connection(|connection| connection.query_row(
            "SELECT workspace.updated_at, scope.created_at, scope.updated_at, scope.generation,
                    desired.created_at, desired.updated_at
             FROM workspaces workspace JOIN effect_scopes scope ON scope.workspace_id = workspace.id
             JOIN effect_desired_effects desired ON desired.scope_id = scope.id
             WHERE workspace.id <> 'workspace-1'",
            [], |row| (0..6).map(|index| row.get::<_, i64>(index)).collect::<Result<Vec<_>, _>>()).map_err(Into::into))?;
        assert_eq!(seeded, vec![10, 300, 300, 1, 300, 300]);
    }
    Ok(())
}
