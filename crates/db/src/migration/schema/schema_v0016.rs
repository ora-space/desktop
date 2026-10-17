use super::Migration;

const UP_STATEMENTS: &[&str] = &[
    r#"
CREATE TABLE effect_target_status_staging_v0016 (
    target_id TEXT PRIMARY KEY,
    desired_generation INTEGER NOT NULL,
    observed_generation INTEGER NOT NULL,
    applied_generation INTEGER NOT NULL,
    ready_generation INTEGER NOT NULL,
    phase TEXT NOT NULL,
    recovery_operation_id TEXT,
    status_version INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO effect_target_status_staging_v0016 (
    target_id, desired_generation, observed_generation, applied_generation,
    ready_generation, phase, recovery_operation_id, status_version, created_at, updated_at
)
SELECT target_id, desired_generation, observed_generation, applied_generation,
       ready_generation, phase, recovery_operation_id, status_version, created_at, updated_at
FROM effect_target_status;

CREATE TABLE effect_resource_status_staging_v0016 (
    resource_id TEXT PRIMARY KEY,
    desired_generation INTEGER NOT NULL,
    observed_generation INTEGER NOT NULL,
    applied_generation INTEGER NOT NULL,
    phase TEXT NOT NULL,
    recovery_operation_id TEXT,
    status_version INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO effect_resource_status_staging_v0016 (
    resource_id, desired_generation, observed_generation, applied_generation,
    phase, recovery_operation_id, status_version, created_at, updated_at
)
SELECT resource_id, desired_generation, observed_generation, applied_generation,
       phase, recovery_operation_id, status_version, created_at, updated_at
FROM effect_resource_status;

CREATE TABLE effect_operation_artifacts_staging_v0016 (
    id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL,
    role TEXT NOT NULL,
    locator_version INTEGER NOT NULL,
    locator_json TEXT NOT NULL,
    expected_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO effect_operation_artifacts_staging_v0016 (
    id, operation_id, role, locator_version, locator_json,
    expected_fingerprint, state, created_at, updated_at
)
SELECT id, operation_id, role, locator_version, locator_json,
       expected_fingerprint, state, created_at, updated_at
FROM effect_operation_artifacts;

CREATE TABLE effect_operations_v0016 (
    id TEXT PRIMARY KEY,
    attempt_id TEXT NOT NULL REFERENCES effect_reconcile_attempts(id),
    resource_id TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 0),
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    mutation TEXT NOT NULL CHECK (mutation IN ('create', 'update', 'replace', 'delete')),
    expected_version INTEGER NOT NULL CHECK (expected_version > 0),
    expected_json TEXT NOT NULL,
    planned_version INTEGER NOT NULL CHECK (planned_version > 0),
    planned_json TEXT NOT NULL,
    payload_version INTEGER NOT NULL CHECK (payload_version > 0),
    payload_json TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN (
        'prepared', 'applied', 'finalized', 'finalized_without_effect', 'recovery_required'
    )),
    prepared_at INTEGER NOT NULL,
    applied_at INTEGER,
    finalized_at INTEGER,
    detected_at INTEGER,
    updated_at INTEGER NOT NULL CHECK (updated_at >= prepared_at),
    UNIQUE (attempt_id, sequence),
    CHECK (applied_at IS NULL OR applied_at >= prepared_at),
    CHECK (finalized_at IS NULL OR phase = 'finalized_without_effect'
           OR (applied_at IS NOT NULL AND finalized_at >= applied_at)),
    CHECK (detected_at IS NULL OR detected_at >= prepared_at),
    CHECK ((phase IN ('prepared', 'applied', 'recovery_required')) = (finalized_at IS NULL)),
    CHECK (phase <> 'finalized' OR applied_at IS NOT NULL)
);

INSERT INTO effect_operations_v0016 (
    id, attempt_id, resource_id, generation, sequence, mutation,
    expected_version, expected_json, planned_version, planned_json,
    payload_version, payload_json, phase, prepared_at, applied_at,
    finalized_at, detected_at, updated_at
)
SELECT id, attempt_id, resource_id, generation, sequence, mutation,
       expected_version, expected_json, planned_version, planned_json,
       payload_version, payload_json, phase, prepared_at, applied_at,
       finalized_at, detected_at, updated_at
FROM effect_operations;

DROP TABLE effect_operation_artifacts;
DROP TABLE effect_target_status;
DROP TABLE effect_resource_status;
DROP TABLE effect_operations;

ALTER TABLE effect_operations_v0016 RENAME TO effect_operations;

CREATE TABLE effect_target_status (
    target_id TEXT PRIMARY KEY REFERENCES effect_targets(id) ON DELETE CASCADE,
    desired_generation INTEGER NOT NULL CHECK (desired_generation >= 0),
    observed_generation INTEGER NOT NULL CHECK (observed_generation >= 0),
    applied_generation INTEGER NOT NULL CHECK (applied_generation >= 0),
    ready_generation INTEGER NOT NULL CHECK (ready_generation >= 0),
    phase TEXT NOT NULL CHECK (phase IN (
        'pending', 'planning', 'coordinating', 'applying', 'verifying', 'activating',
        'current', 'current_with_issues', 'retiring', 'recovery_required'
    )),
    recovery_operation_id TEXT REFERENCES effect_operations(id),
    status_version INTEGER NOT NULL CHECK (status_version > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (ready_generation <= applied_generation),
    CHECK (applied_generation <= observed_generation),
    CHECK (observed_generation <= desired_generation),
    CHECK ((phase = 'recovery_required') = (recovery_operation_id IS NOT NULL))
);

INSERT INTO effect_target_status (
    target_id, desired_generation, observed_generation, applied_generation,
    ready_generation, phase, recovery_operation_id, status_version, created_at, updated_at
)
SELECT target_id, desired_generation, observed_generation, applied_generation,
       ready_generation, phase, recovery_operation_id, status_version, created_at, updated_at
FROM effect_target_status_staging_v0016;

CREATE TABLE effect_resource_status (
    resource_id TEXT PRIMARY KEY REFERENCES effect_resources(id) ON DELETE CASCADE,
    desired_generation INTEGER NOT NULL CHECK (desired_generation >= 0),
    observed_generation INTEGER NOT NULL CHECK (observed_generation >= 0),
    applied_generation INTEGER NOT NULL CHECK (applied_generation >= 0),
    phase TEXT NOT NULL CHECK (phase IN (
        'pending', 'reconciling', 'current', 'retiring', 'recovery_required'
    )),
    recovery_operation_id TEXT REFERENCES effect_operations(id),
    status_version INTEGER NOT NULL CHECK (status_version > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (applied_generation <= observed_generation),
    CHECK (observed_generation <= desired_generation),
    CHECK ((phase = 'recovery_required') = (recovery_operation_id IS NOT NULL))
);

INSERT INTO effect_resource_status (
    resource_id, desired_generation, observed_generation, applied_generation,
    phase, recovery_operation_id, status_version, created_at, updated_at
)
SELECT resource_id, desired_generation, observed_generation, applied_generation,
       phase, recovery_operation_id, status_version, created_at, updated_at
FROM effect_resource_status_staging_v0016;

CREATE TABLE effect_operation_artifacts (
    id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL REFERENCES effect_operations(id),
    role TEXT NOT NULL,
    locator_version INTEGER NOT NULL CHECK (locator_version > 0),
    locator_json TEXT NOT NULL,
    expected_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'reserved', 'retained', 'pending_cleanup', 'cleanup_failed'
    )),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (operation_id, role, locator_version, locator_json)
);

INSERT INTO effect_operation_artifacts (
    id, operation_id, role, locator_version, locator_json,
    expected_fingerprint, state, created_at, updated_at
)
SELECT id, operation_id, role, locator_version, locator_json,
       expected_fingerprint, state, created_at, updated_at
FROM effect_operation_artifacts_staging_v0016;

DROP TABLE effect_target_status_staging_v0016;
DROP TABLE effect_resource_status_staging_v0016;
DROP TABLE effect_operation_artifacts_staging_v0016;

CREATE INDEX effect_operations_recovery
    ON effect_operations(resource_id, prepared_at, id)
    WHERE phase NOT IN ('finalized', 'finalized_without_effect');

CREATE TRIGGER effect_operations_intent_immutable
BEFORE UPDATE OF attempt_id, resource_id, generation, sequence, mutation, expected_version,
                 expected_json, planned_version, planned_json, payload_version, payload_json
ON effect_operations
BEGIN
    SELECT RAISE(ABORT, 'Effect operation intent is immutable');
END;

CREATE TRIGGER effect_operations_recovery_time_insert
BEFORE INSERT ON effect_operations
WHEN (NEW.phase = 'recovery_required') != (NEW.detected_at IS NOT NULL)
  OR NEW.detected_at < NEW.prepared_at
BEGIN
    SELECT RAISE(ABORT, 'invalid Effect recovery detection time');
END;

CREATE TRIGGER effect_operations_recovery_time_update
BEFORE UPDATE ON effect_operations
WHEN (NEW.phase = 'recovery_required') != (NEW.detected_at IS NOT NULL)
  OR NEW.detected_at < NEW.prepared_at
BEGIN
    SELECT RAISE(ABORT, 'invalid Effect recovery detection time');
END;

CREATE INDEX effect_operation_artifacts_cleanup
    ON effect_operation_artifacts(state, updated_at, id)
    WHERE state IN ('pending_cleanup', 'cleanup_failed');

CREATE TRIGGER effect_operation_artifacts_authority_immutable
BEFORE UPDATE OF operation_id, role, locator_version, locator_json, expected_fingerprint
ON effect_operation_artifacts
BEGIN
    SELECT RAISE(ABORT, 'Effect operation artifact authority is immutable');
END;
"#,
];

const DOWN_STATEMENTS: &[&str] = &[
    r#"
-- A closed-without-effect journal cannot exist in the pre-recovery schema: its evidence
-- stays intact by returning the row to explicit manual recovery, which a later upgrade
-- can prove and close again. The pre-0016 application never claims a Target with
-- unfinished operations, so the rescheduled request simply waits instead of guessing.
UPDATE effect_operations
SET phase = 'recovery_required',
    finalized_at = NULL,
    detected_at = COALESCE(detected_at, updated_at)
WHERE phase = 'finalized_without_effect';

CREATE TABLE effect_target_status_staging_v0016 (
    target_id TEXT PRIMARY KEY,
    desired_generation INTEGER NOT NULL,
    observed_generation INTEGER NOT NULL,
    applied_generation INTEGER NOT NULL,
    ready_generation INTEGER NOT NULL,
    phase TEXT NOT NULL,
    recovery_operation_id TEXT,
    status_version INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO effect_target_status_staging_v0016 (
    target_id, desired_generation, observed_generation, applied_generation,
    ready_generation, phase, recovery_operation_id, status_version, created_at, updated_at
)
SELECT target_id, desired_generation, observed_generation, applied_generation,
       ready_generation, phase, recovery_operation_id, status_version, created_at, updated_at
FROM effect_target_status;

CREATE TABLE effect_resource_status_staging_v0016 (
    resource_id TEXT PRIMARY KEY,
    desired_generation INTEGER NOT NULL,
    observed_generation INTEGER NOT NULL,
    applied_generation INTEGER NOT NULL,
    phase TEXT NOT NULL,
    recovery_operation_id TEXT,
    status_version INTEGER NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO effect_resource_status_staging_v0016 (
    resource_id, desired_generation, observed_generation, applied_generation,
    phase, recovery_operation_id, status_version, created_at, updated_at
)
SELECT resource_id, desired_generation, observed_generation, applied_generation,
       phase, recovery_operation_id, status_version, created_at, updated_at
FROM effect_resource_status;

CREATE TABLE effect_operation_artifacts_staging_v0016 (
    id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL,
    role TEXT NOT NULL,
    locator_version INTEGER NOT NULL,
    locator_json TEXT NOT NULL,
    expected_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

INSERT INTO effect_operation_artifacts_staging_v0016 (
    id, operation_id, role, locator_version, locator_json,
    expected_fingerprint, state, created_at, updated_at
)
SELECT id, operation_id, role, locator_version, locator_json,
       expected_fingerprint, state, created_at, updated_at
FROM effect_operation_artifacts;

CREATE TABLE effect_operations_v0015 (
    id TEXT PRIMARY KEY,
    attempt_id TEXT NOT NULL REFERENCES effect_reconcile_attempts(id),
    resource_id TEXT NOT NULL,
    generation INTEGER NOT NULL CHECK (generation >= 0),
    sequence INTEGER NOT NULL CHECK (sequence >= 0),
    mutation TEXT NOT NULL CHECK (mutation IN ('create', 'update', 'replace', 'delete')),
    expected_version INTEGER NOT NULL CHECK (expected_version > 0),
    expected_json TEXT NOT NULL,
    planned_version INTEGER NOT NULL CHECK (planned_version > 0),
    planned_json TEXT NOT NULL,
    payload_version INTEGER NOT NULL CHECK (payload_version > 0),
    payload_json TEXT NOT NULL,
    phase TEXT NOT NULL CHECK (phase IN ('prepared', 'applied', 'finalized', 'recovery_required')),
    prepared_at INTEGER NOT NULL,
    applied_at INTEGER,
    finalized_at INTEGER,
    updated_at INTEGER NOT NULL CHECK (updated_at >= prepared_at),
    detected_at INTEGER,
    UNIQUE (attempt_id, sequence),
    CHECK (applied_at IS NULL OR applied_at >= prepared_at),
    CHECK (finalized_at IS NULL OR (applied_at IS NOT NULL AND finalized_at >= applied_at))
);

INSERT INTO effect_operations_v0015 (
    id, attempt_id, resource_id, generation, sequence, mutation,
    expected_version, expected_json, planned_version, planned_json,
    payload_version, payload_json, phase, prepared_at, applied_at,
    finalized_at, updated_at, detected_at
)
SELECT id, attempt_id, resource_id, generation, sequence, mutation,
       expected_version, expected_json, planned_version, planned_json,
       payload_version, payload_json, phase, prepared_at, applied_at,
       finalized_at, updated_at, detected_at
FROM effect_operations;

DROP TABLE effect_operation_artifacts;
DROP TABLE effect_target_status;
DROP TABLE effect_resource_status;
DROP TABLE effect_operations;

ALTER TABLE effect_operations_v0015 RENAME TO effect_operations;

CREATE TABLE effect_target_status (
    target_id TEXT PRIMARY KEY REFERENCES effect_targets(id) ON DELETE CASCADE,
    desired_generation INTEGER NOT NULL CHECK (desired_generation >= 0),
    observed_generation INTEGER NOT NULL CHECK (observed_generation >= 0),
    applied_generation INTEGER NOT NULL CHECK (applied_generation >= 0),
    ready_generation INTEGER NOT NULL CHECK (ready_generation >= 0),
    phase TEXT NOT NULL CHECK (phase IN (
        'pending', 'planning', 'coordinating', 'applying', 'verifying', 'activating',
        'current', 'current_with_issues', 'retiring', 'recovery_required'
    )),
    recovery_operation_id TEXT REFERENCES effect_operations(id),
    status_version INTEGER NOT NULL CHECK (status_version > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (ready_generation <= applied_generation),
    CHECK (applied_generation <= observed_generation),
    CHECK (observed_generation <= desired_generation),
    CHECK ((phase = 'recovery_required') = (recovery_operation_id IS NOT NULL))
);

INSERT INTO effect_target_status (
    target_id, desired_generation, observed_generation, applied_generation,
    ready_generation, phase, recovery_operation_id, status_version, created_at, updated_at
)
SELECT target_id, desired_generation, observed_generation, applied_generation,
       ready_generation, phase, recovery_operation_id, status_version, created_at, updated_at
FROM effect_target_status_staging_v0016;

CREATE TABLE effect_resource_status (
    resource_id TEXT PRIMARY KEY REFERENCES effect_resources(id) ON DELETE CASCADE,
    desired_generation INTEGER NOT NULL CHECK (desired_generation >= 0),
    observed_generation INTEGER NOT NULL CHECK (observed_generation >= 0),
    applied_generation INTEGER NOT NULL CHECK (applied_generation >= 0),
    phase TEXT NOT NULL CHECK (phase IN (
        'pending', 'reconciling', 'current', 'retiring', 'recovery_required'
    )),
    recovery_operation_id TEXT REFERENCES effect_operations(id),
    status_version INTEGER NOT NULL CHECK (status_version > 0),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    CHECK (applied_generation <= observed_generation),
    CHECK (observed_generation <= desired_generation),
    CHECK ((phase = 'recovery_required') = (recovery_operation_id IS NOT NULL))
);

INSERT INTO effect_resource_status (
    resource_id, desired_generation, observed_generation, applied_generation,
    phase, recovery_operation_id, status_version, created_at, updated_at
)
SELECT resource_id, desired_generation, observed_generation, applied_generation,
       phase, recovery_operation_id, status_version, created_at, updated_at
FROM effect_resource_status_staging_v0016;

CREATE TABLE effect_operation_artifacts (
    id TEXT PRIMARY KEY,
    operation_id TEXT NOT NULL REFERENCES effect_operations(id),
    role TEXT NOT NULL,
    locator_version INTEGER NOT NULL CHECK (locator_version > 0),
    locator_json TEXT NOT NULL,
    expected_fingerprint TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN (
        'reserved', 'retained', 'pending_cleanup', 'cleanup_failed'
    )),
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL CHECK (updated_at >= created_at),
    UNIQUE (operation_id, role, locator_version, locator_json)
);

INSERT INTO effect_operation_artifacts (
    id, operation_id, role, locator_version, locator_json,
    expected_fingerprint, state, created_at, updated_at
)
SELECT id, operation_id, role, locator_version, locator_json,
       expected_fingerprint, state, created_at, updated_at
FROM effect_operation_artifacts_staging_v0016;

DROP TABLE effect_target_status_staging_v0016;
DROP TABLE effect_resource_status_staging_v0016;
DROP TABLE effect_operation_artifacts_staging_v0016;

CREATE INDEX effect_operations_recovery
    ON effect_operations(resource_id, prepared_at, id) WHERE phase <> 'finalized';

CREATE TRIGGER effect_operations_intent_immutable
BEFORE UPDATE OF attempt_id, resource_id, generation, sequence, mutation, expected_version,
                 expected_json, planned_version, planned_json, payload_version, payload_json
ON effect_operations
BEGIN
    SELECT RAISE(ABORT, 'Effect operation intent is immutable');
END;

CREATE TRIGGER effect_operations_recovery_time_insert
BEFORE INSERT ON effect_operations
WHEN (NEW.phase = 'recovery_required') != (NEW.detected_at IS NOT NULL)
  OR NEW.detected_at < NEW.prepared_at
BEGIN
    SELECT RAISE(ABORT, 'invalid Effect recovery detection time');
END;

CREATE TRIGGER effect_operations_recovery_time_update
BEFORE UPDATE ON effect_operations
WHEN (NEW.phase = 'recovery_required') != (NEW.detected_at IS NOT NULL)
  OR NEW.detected_at < NEW.prepared_at
BEGIN
    SELECT RAISE(ABORT, 'invalid Effect recovery detection time');
END;

CREATE INDEX effect_operation_artifacts_cleanup
    ON effect_operation_artifacts(state, updated_at, id)
    WHERE state IN ('pending_cleanup', 'cleanup_failed');

CREATE TRIGGER effect_operation_artifacts_authority_immutable
BEFORE UPDATE OF operation_id, role, locator_version, locator_json, expected_fingerprint
ON effect_operation_artifacts
BEGIN
    SELECT RAISE(ABORT, 'Effect operation artifact authority is immutable');
END;
"#,
];

/// Adds the `finalized_without_effect` operation phase used by the explicit recovery exit.
///
/// Recovery closes a quarantined journal whose exact expected baseline was proven: the
/// planned effect never became durable, so the row keeps its journal evidence while
/// recording that no ownership was granted. The old `finalized_at` CHECK required an
/// `applied_at` value that a closed-without-effect row cannot honestly provide, so the
/// journal tables are rebuilt through staging copies that preserve every row and
/// foreign-key reference; the referencing status and artifact tables are recreated only
/// after the rebuilt operations table owns its final name.
pub fn migration() -> Migration {
    Migration::new("0016", UP_STATEMENTS, DOWN_STATEMENTS)
}

#[cfg(test)]
mod tests {
    use super::{DOWN_STATEMENTS, UP_STATEMENTS};
    use rusqlite::{Connection, params};

    /// Builds the pre-0016 Effect journal schema with rows in every durable phase,
    /// including a quarantined Target and Resource that reference a recovery operation.
    fn fixture() -> Connection {
        let connection = Connection::open_in_memory().expect("open database");
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .expect("enable foreign keys");
        connection
            .execute_batch(
                r#"
                CREATE TABLE effect_reconcile_attempts (
                    id TEXT PRIMARY KEY,
                    target_id TEXT NOT NULL,
                    generation INTEGER NOT NULL CHECK (generation >= 0),
                    consumer_revision_id TEXT NOT NULL,
                    target_projection_digest TEXT NOT NULL,
                    coordination_plan_version INTEGER NOT NULL CHECK (coordination_plan_version > 0),
                    coordination_plan_json TEXT NOT NULL,
                    phase TEXT NOT NULL,
                    prepared_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL CHECK (updated_at >= prepared_at)
                );
                CREATE TABLE effect_targets (
                    id TEXT PRIMARY KEY
                );
                CREATE TABLE effect_resources (
                    id TEXT PRIMARY KEY
                );
                CREATE TABLE effect_operations (
                    id TEXT PRIMARY KEY,
                    attempt_id TEXT NOT NULL REFERENCES effect_reconcile_attempts(id),
                    resource_id TEXT NOT NULL,
                    generation INTEGER NOT NULL CHECK (generation >= 0),
                    sequence INTEGER NOT NULL CHECK (sequence >= 0),
                    mutation TEXT NOT NULL CHECK (mutation IN ('create', 'update', 'replace', 'delete')),
                    expected_version INTEGER NOT NULL CHECK (expected_version > 0),
                    expected_json TEXT NOT NULL,
                    planned_version INTEGER NOT NULL CHECK (planned_version > 0),
                    planned_json TEXT NOT NULL,
                    payload_version INTEGER NOT NULL CHECK (payload_version > 0),
                    payload_json TEXT NOT NULL,
                    phase TEXT NOT NULL CHECK (phase IN ('prepared', 'applied', 'finalized', 'recovery_required')),
                    prepared_at INTEGER NOT NULL,
                    applied_at INTEGER,
                    finalized_at INTEGER,
                    detected_at INTEGER,
                    updated_at INTEGER NOT NULL CHECK (updated_at >= prepared_at),
                    UNIQUE (attempt_id, sequence),
                    CHECK (applied_at IS NULL OR applied_at >= prepared_at),
                    CHECK (finalized_at IS NULL OR (applied_at IS NOT NULL AND finalized_at >= applied_at))
                );
                CREATE TABLE effect_target_status (
                    target_id TEXT PRIMARY KEY REFERENCES effect_targets(id) ON DELETE CASCADE,
                    desired_generation INTEGER NOT NULL,
                    observed_generation INTEGER NOT NULL,
                    applied_generation INTEGER NOT NULL,
                    ready_generation INTEGER NOT NULL,
                    phase TEXT NOT NULL,
                    recovery_operation_id TEXT REFERENCES effect_operations(id),
                    status_version INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE effect_resource_status (
                    resource_id TEXT PRIMARY KEY REFERENCES effect_resources(id) ON DELETE CASCADE,
                    desired_generation INTEGER NOT NULL,
                    observed_generation INTEGER NOT NULL,
                    applied_generation INTEGER NOT NULL,
                    phase TEXT NOT NULL,
                    recovery_operation_id TEXT REFERENCES effect_operations(id),
                    status_version INTEGER NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL
                );
                CREATE TABLE effect_operation_artifacts (
                    id TEXT PRIMARY KEY,
                    operation_id TEXT NOT NULL REFERENCES effect_operations(id),
                    role TEXT NOT NULL,
                    locator_version INTEGER NOT NULL,
                    locator_json TEXT NOT NULL,
                    expected_fingerprint TEXT NOT NULL,
                    state TEXT NOT NULL,
                    created_at INTEGER NOT NULL,
                    updated_at INTEGER NOT NULL,
                    UNIQUE (operation_id, role, locator_version, locator_json)
                );
                INSERT INTO effect_reconcile_attempts VALUES (
                    'attempt-1', 'target-1', 1, 'revision-1', 'digest-1', 1, '{}',
                    'recovery_required', 5, 5
                );
                INSERT INTO effect_operations (
                    id, attempt_id, resource_id, generation, sequence, mutation,
                    expected_version, expected_json, planned_version, planned_json,
                    payload_version, payload_json, phase, prepared_at, applied_at,
                    finalized_at, detected_at, updated_at
                ) VALUES
                    ('operation-quarantined', 'attempt-1', 'resource-1', 1, 0, 'create',
                     1, '{}', 1, '{}', 1, '{}', 'recovery_required', 5, 6, NULL, 6, 6),
                    ('operation-finalized', 'attempt-1', 'resource-1', 1, 1, 'create',
                     1, '{}', 1, '{}', 1, '{}', 'finalized', 5, 6, 7, NULL, 7);
                INSERT INTO effect_targets VALUES ('target-1');
                INSERT INTO effect_resources VALUES ('resource-1');
                INSERT INTO effect_target_status VALUES (
                    'target-1', 1, 1, 0, 0, 'recovery_required', 'operation-quarantined', 3, 1, 6
                );
                INSERT INTO effect_resource_status VALUES (
                    'resource-1', 1, 1, 0, 'recovery_required', 'operation-quarantined', 3, 1, 6
                );
                INSERT INTO effect_operation_artifacts VALUES (
                    'artifact-1', 'operation-quarantined', 'staging', 1, '{}', 'fp', 'retained', 5, 6
                );
                "#,
            )
            .expect("create pre-0016 Effect schema fixture");
        connection
    }

    /// Rebuilding keeps every journal row and foreign-key reference, accepts the new
    /// closed-without-effect phase, and rejects it again after downgrade.
    #[test]
    fn rebuild_preserves_rows_and_enables_closed_without_effect_phase() {
        let connection = fixture();
        for statement in UP_STATEMENTS {
            connection.execute_batch(statement).expect("apply upgrade");
        }

        let phases: Vec<String> = connection
            .prepare("SELECT phase FROM effect_operations ORDER BY id")
            .unwrap()
            .query_map([], |row| row.get(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            phases,
            vec!["finalized".to_string(), "recovery_required".to_string()]
        );
        let quarantined_status: (String, Option<String>) = connection
            .query_row(
                "SELECT phase, recovery_operation_id FROM effect_target_status",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            quarantined_status,
            (
                "recovery_required".to_string(),
                Some("operation-quarantined".to_string())
            )
        );

        // The recovery exit can persist a closure without apply evidence while its
        // quarantine detection time is cleared; a Prepared-only journal keeps applied_at
        // NULL because its mutation provably never ran.
        connection
            .execute(
                "UPDATE effect_operations
                 SET phase = 'finalized_without_effect', finalized_at = 9, applied_at = NULL,
                     detected_at = NULL, updated_at = 9
                 WHERE id = 'operation-quarantined'",
                [],
            )
            .expect("close a quarantined journal without effect");
        let closed: (String, Option<i64>, Option<i64>, Option<i64>) = connection
            .query_row(
                "SELECT phase, applied_at, finalized_at, detected_at
                 FROM effect_operations WHERE id = 'operation-quarantined'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            closed,
            (
                "finalized_without_effect".to_string(),
                None,
                Some(9),
                None
            )
        );
        // The schema still refuses a finalized row without apply evidence.
        assert!(connection
            .execute(
                "UPDATE effect_operations SET phase = 'finalized' WHERE id = 'operation-quarantined'",
                [],
            )
            .is_err());

        // Downgrade maps the closure back to explicit recovery and restores the old CHECKs.
        for statement in DOWN_STATEMENTS {
            connection.execute_batch(statement).expect("apply downgrade");
        }
        let reverted: (String, Option<i64>, Option<i64>, Option<i64>) = connection
            .query_row(
                "SELECT phase, finalized_at, applied_at, detected_at
                 FROM effect_operations WHERE id = 'operation-quarantined'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
        assert_eq!(
            reverted,
            ("recovery_required".to_string(), None, None, Some(9))
        );
        assert!(connection
            .execute(
                "UPDATE effect_operations SET phase = 'finalized_without_effect'
                 WHERE id = 'operation-quarantined'",
                [],
            )
            .is_err());
        let counts: (i64, i64, i64) = connection
            .query_row(
                "SELECT
                     (SELECT COUNT(*) FROM effect_operations),
                     (SELECT COUNT(*) FROM effect_target_status),
                     (SELECT COUNT(*) FROM effect_operation_artifacts)
                 FROM (SELECT 1)",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(counts, (2, 1, 1));
        // Foreign keys still guard the rebuilt tables after the round trip.
        assert!(connection
            .execute(
                "INSERT INTO effect_operation_artifacts VALUES (
                     'artifact-2', 'missing-operation', 'staging', 1, '{}', 'fp', 'retained', 5, 6
                 )",
                params![],
            )
            .is_err());
    }

    /// Re-running the upgrade after a downgrade is idempotent for the row content.
    #[test]
    fn upgrade_downgrade_upgrade_cycle_retains_journal_evidence() {
        let connection = fixture();
        for statement in UP_STATEMENTS {
            connection.execute_batch(statement).expect("apply upgrade");
        }
        connection
            .execute(
                "UPDATE effect_operations
                 SET phase = 'finalized_without_effect', finalized_at = 9, applied_at = NULL,
                     detected_at = NULL, updated_at = 9
                 WHERE id = 'operation-quarantined'",
                params![],
            )
            .expect("close the quarantined journal");
        for statement in DOWN_STATEMENTS {
            connection.execute_batch(statement).expect("apply downgrade");
        }
        for statement in UP_STATEMENTS {
            connection.execute_batch(statement).expect("re-apply upgrade");
        }
        let phase: String = connection
            .query_row(
                "SELECT phase FROM effect_operations WHERE id = 'operation-quarantined'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(phase, "recovery_required");
    }
}
