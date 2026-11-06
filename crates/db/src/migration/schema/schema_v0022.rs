use super::Migration;

// Remote locations keep `provisioning`: a provisioner does exist for them. Provisioning rows are
// released first because the second statement changes the lifecycle both statements select on.
const UP_STATEMENTS: &[&str] = &[
    "UPDATE workspace_provisioning SET state = 'ready'
     WHERE provisioner_kind = 'local_git' AND state = 'pending'
       AND workspace_id IN (
           SELECT w.id
           FROM workspaces w
           JOIN workspace_locations l ON l.id = w.location_id
           WHERE w.workspace_kind = 'main' AND w.lifecycle = 'provisioning'
             AND w.is_deleted = 0 AND l.location_kind = 'local_filesystem'
       );",
    "UPDATE workspaces SET lifecycle = 'active'
     WHERE workspace_kind = 'main' AND lifecycle = 'provisioning' AND is_deleted = 0
       AND location_id IN (
           SELECT id FROM workspace_locations WHERE location_kind = 'local_filesystem'
       );",
];

/// Intentionally empty: an older build admits an active local Workspace exactly as this one does,
/// and restoring `provisioning` would strand those projects again.
const DOWN_STATEMENTS: &[&str] = &[];

/// Releases local main Workspaces stranded in `provisioning` by earlier project creation.
///
/// Creation used to probe the directory once and store `provisioning` when it was absent, but no
/// code ever promotes a local Workspace, so such a project could never list models or start a
/// session even after its directory appeared. Local Workspaces are now always active and their
/// directory is checked on every use, so the stranded rows only need their state corrected.
pub fn migration() -> Migration {
    Migration::new("0022", UP_STATEMENTS, DOWN_STATEMENTS)
}

#[cfg(test)]
mod tests {
    use crate::migration::reconcile_database_versions;
    use crate::{MigrationCatalog, default_migration_catalog, test_clock::TestClock};
    use ora_logging::with_trace_logging;
    use pretty_assertions::assert_eq;
    use rusqlite::Connection;

    /// Promotes only stranded local main Workspaces and leaves every other row untouched.
    #[test]
    fn releases_stranded_local_main_workspaces_only() {
        with_trace_logging(|| {
            let latest = default_migration_catalog().unwrap();
            let previous = MigrationCatalog::new(
                latest
                    .target_versions()
                    .iter()
                    .filter(|version| **version < "0022")
                    .map(|version| latest.migration(version).unwrap().clone())
                    .collect(),
            )
            .unwrap();
            let mut connection = Connection::open_in_memory().unwrap();
            reconcile_database_versions(&mut connection, &previous, &TestClock::new(1)).unwrap();
            connection
                .execute_batch(
                    "INSERT INTO projects (id, name, created_at, updated_at, is_deleted) VALUES
                         ('stranded', 'Stranded', 1, 1, 0),
                         ('ready', 'Ready', 1, 1, 0),
                         ('remote', 'Remote', 1, 1, 0),
                         ('deleted', 'Deleted', 1, 1, 1);
                     INSERT INTO workspace_locations (id, location_kind, plugin_id, locator_json, created_at, updated_at) VALUES
                         ('l-stranded', 'local_filesystem', NULL, '{\"path\":\"D:\\\\missing\"}', 1, 1),
                         ('l-ready', 'local_filesystem', NULL, '{\"path\":\"D:\\\\ready\"}', 1, 1),
                         ('l-remote', 'ssh', NULL, '{\"connection_ref\":\"c\",\"path\":\"/r\"}', 1, 1),
                         ('l-deleted', 'local_filesystem', NULL, '{\"path\":\"D:\\\\gone\"}', 1, 1);
                     INSERT INTO workspaces (id, project_id, workspace_kind, location_id, lifecycle, created_at, updated_at, is_deleted) VALUES
                         ('w-stranded', 'stranded', 'main', 'l-stranded', 'provisioning', 1, 1, 0),
                         ('w-ready', 'ready', 'main', 'l-ready', 'active', 1, 1, 0),
                         ('w-remote', 'remote', 'main', 'l-remote', 'provisioning', 1, 1, 0),
                         ('w-deleted', 'deleted', 'main', 'l-deleted', 'retiring', 1, 1, 1);
                     INSERT INTO workspace_provisioning (workspace_id, provisioner_kind, state, created_at, updated_at) VALUES
                         ('w-stranded', 'local_git', 'pending', 1, 1),
                         ('w-ready', 'local_git', 'ready', 1, 1),
                         ('w-remote', 'ssh', 'pending', 1, 1),
                         ('w-deleted', 'local_git', 'destroying', 1, 1);",
                )
                .unwrap();

            reconcile_database_versions(&mut connection, &latest, &TestClock::new(2)).unwrap();

            let rows = connection
                .prepare(
                    "SELECT w.id, w.lifecycle, p.state
                     FROM workspaces w JOIN workspace_provisioning p ON p.workspace_id = w.id
                     ORDER BY w.id",
                )
                .unwrap()
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
                .unwrap()
                .collect::<Result<Vec<(String, String, String)>, _>>()
                .unwrap();
            assert_eq!(
                rows,
                [
                    ("w-deleted", "retiring", "destroying"),
                    ("w-ready", "active", "ready"),
                    ("w-remote", "provisioning", "pending"),
                    ("w-stranded", "active", "ready"),
                ]
                .map(|(id, lifecycle, state)| (
                    id.into(),
                    lifecycle.into(),
                    state.into()
                ))
            );
        });
    }
}
