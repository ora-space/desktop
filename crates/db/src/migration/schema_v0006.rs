use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
CREATE TABLE effect_sources (
    id TEXT PRIMARY KEY, effect_kind TEXT NOT NULL, source_kind TEXT NOT NULL,
    namespace TEXT NOT NULL COLLATE NOCASE, identifier TEXT NOT NULL COLLATE NOCASE,
    lifecycle TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    UNIQUE (effect_kind, source_kind, namespace, identifier), CHECK (updated_at >= created_at)
);
CREATE TABLE effect_source_revisions (
    id TEXT PRIMARY KEY, source_id TEXT NOT NULL REFERENCES effect_sources(id),
    revision TEXT NOT NULL, state_digest TEXT NOT NULL, payload_json TEXT NOT NULL,
    availability TEXT NOT NULL, unavailable_reason TEXT, created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL, UNIQUE (source_id, id), UNIQUE (source_id, revision),
    CHECK (updated_at >= created_at)
);
CREATE TABLE effect_source_heads (
    source_id TEXT PRIMARY KEY REFERENCES effect_sources(id), revision_id TEXT NOT NULL,
    updated_at INTEGER NOT NULL,
    FOREIGN KEY (source_id, revision_id) REFERENCES effect_source_revisions(source_id, id)
);
CREATE TABLE workspace_effects (
    workspace_id TEXT PRIMARY KEY REFERENCES workspaces(id), generation INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    CHECK (generation >= 0), CHECK (updated_at >= created_at)
);
CREATE TABLE effect_surfaces (
    id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES workspace_effects(workspace_id),
    adapter_kind TEXT NOT NULL, locator_key TEXT NOT NULL, locator_json TEXT NOT NULL,
    format_kind TEXT NOT NULL, lifecycle TEXT NOT NULL, created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL, UNIQUE (workspace_id, adapter_kind, locator_key),
    CHECK (updated_at >= created_at)
);
CREATE TABLE effect_surface_consumers (
    surface_id TEXT NOT NULL REFERENCES effect_surfaces(id) ON DELETE CASCADE,
    consumer_id TEXT NOT NULL, coordination_kind TEXT NOT NULL, created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL, PRIMARY KEY (surface_id, consumer_id),
    CHECK (updated_at >= created_at)
);
CREATE TABLE workspace_effect_desired_items (
    id TEXT PRIMARY KEY, workspace_id TEXT NOT NULL REFERENCES workspace_effects(workspace_id) ON DELETE CASCADE,
    source_id TEXT NOT NULL REFERENCES effect_sources(id), revision_id TEXT NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, UNIQUE (workspace_id, source_id),
    FOREIGN KEY (source_id, revision_id) REFERENCES effect_source_revisions(source_id, id),
    CHECK (updated_at >= created_at)
);
CREATE INDEX workspace_effect_desired_items_source_workspace
    ON workspace_effect_desired_items(source_id, workspace_id);
CREATE TABLE effect_managed_items (
    id TEXT PRIMARY KEY, surface_id TEXT NOT NULL REFERENCES effect_surfaces(id),
    source_id TEXT NOT NULL REFERENCES effect_sources(id), applied_revision_id TEXT NOT NULL,
    target_key TEXT NOT NULL, target_json TEXT NOT NULL, applied_fingerprint TEXT NOT NULL,
    applied_generation INTEGER NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    UNIQUE (surface_id, source_id), UNIQUE (surface_id, target_key),
    FOREIGN KEY (source_id, applied_revision_id) REFERENCES effect_source_revisions(source_id, id),
    CHECK (applied_generation >= 0), CHECK (updated_at >= created_at)
);
CREATE TABLE effect_surface_status (
    surface_id TEXT PRIMARY KEY REFERENCES effect_surfaces(id) ON DELETE CASCADE,
    desired_generation INTEGER NOT NULL DEFAULT 0, observed_generation INTEGER NOT NULL DEFAULT 0,
    applied_generation INTEGER NOT NULL DEFAULT 0, phase TEXT NOT NULL,
    status_version INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    CHECK (desired_generation >= 0), CHECK (observed_generation >= 0),
    CHECK (applied_generation >= 0), CHECK (applied_generation <= observed_generation),
    CHECK (observed_generation <= desired_generation), CHECK (status_version > 0),
    CHECK (updated_at >= created_at)
);
CREATE TABLE effect_consumer_status (
    surface_id TEXT NOT NULL, consumer_id TEXT NOT NULL, ready_generation INTEGER NOT NULL DEFAULT 0,
    phase TEXT NOT NULL, status_version INTEGER NOT NULL DEFAULT 1, created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL, PRIMARY KEY (surface_id, consumer_id),
    FOREIGN KEY (surface_id, consumer_id) REFERENCES effect_surface_consumers(surface_id, consumer_id) ON DELETE CASCADE,
    CHECK (ready_generation >= 0), CHECK (status_version > 0), CHECK (updated_at >= created_at)
);
CREATE TABLE effect_conditions (
    id TEXT PRIMARY KEY, surface_id TEXT NOT NULL REFERENCES effect_surface_status(surface_id) ON DELETE CASCADE,
    consumer_id TEXT, subject_kind TEXT NOT NULL, subject_id TEXT NOT NULL, reason TEXT NOT NULL,
    failed_generation INTEGER, message TEXT NOT NULL, first_observed_at INTEGER NOT NULL,
    last_observed_at INTEGER NOT NULL,
    FOREIGN KEY (surface_id, consumer_id) REFERENCES effect_consumer_status(surface_id, consumer_id) ON DELETE CASCADE,
    CHECK (failed_generation IS NULL OR failed_generation >= 0),
    CHECK (last_observed_at >= first_observed_at)
);
CREATE UNIQUE INDEX effect_conditions_surface_unique
    ON effect_conditions(surface_id, subject_kind, subject_id, reason) WHERE consumer_id IS NULL;
CREATE UNIQUE INDEX effect_conditions_consumer_unique
    ON effect_conditions(surface_id, consumer_id, subject_kind, subject_id, reason) WHERE consumer_id IS NOT NULL;
CREATE TABLE effect_reconcile_requests (
    surface_id TEXT PRIMARY KEY REFERENCES effect_surfaces(id) ON DELETE CASCADE,
    requested_generation INTEGER NOT NULL, request_token TEXT NOT NULL,
    attempt_count INTEGER NOT NULL DEFAULT 0, requested_at INTEGER NOT NULL,
    not_before_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    CHECK (requested_generation >= 0), CHECK (attempt_count >= 0),
    CHECK (not_before_at >= requested_at), CHECK (updated_at >= requested_at)
);
CREATE INDEX effect_reconcile_requests_due
    ON effect_reconcile_requests(not_before_at, requested_at, surface_id);
CREATE TABLE effect_propagation_requests (
    source_id TEXT PRIMARY KEY REFERENCES effect_sources(id) ON DELETE CASCADE,
    head_revision_id TEXT NOT NULL, request_token TEXT NOT NULL, attempt_count INTEGER NOT NULL DEFAULT 0,
    requested_at INTEGER NOT NULL, not_before_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    FOREIGN KEY (source_id, head_revision_id) REFERENCES effect_source_revisions(source_id, id),
    CHECK (attempt_count >= 0), CHECK (not_before_at >= requested_at), CHECK (updated_at >= requested_at)
);
CREATE INDEX effect_propagation_requests_due
    ON effect_propagation_requests(not_before_at, requested_at, source_id);
CREATE TABLE effect_operations (
    id TEXT PRIMARY KEY, surface_id TEXT NOT NULL REFERENCES effect_surfaces(id), generation INTEGER NOT NULL,
    target_key TEXT NOT NULL, operation_kind TEXT NOT NULL, phase TEXT NOT NULL,
    payload_version INTEGER NOT NULL, payload_json TEXT NOT NULL, prepared_at INTEGER NOT NULL,
    applied_at INTEGER, finalized_at INTEGER, updated_at INTEGER NOT NULL,
    CHECK (generation >= 0), CHECK (payload_version > 0),
    CHECK (applied_at IS NULL OR applied_at >= prepared_at),
    CHECK (finalized_at IS NULL OR (applied_at IS NOT NULL AND finalized_at >= applied_at)),
    CHECK (updated_at >= prepared_at), CHECK (applied_at IS NULL OR updated_at >= applied_at),
    CHECK (finalized_at IS NULL OR updated_at >= finalized_at)
);
CREATE UNIQUE INDEX effect_operations_active_target_unique
    ON effect_operations(surface_id, target_key) WHERE phase <> 'finalized';
CREATE INDEX effect_operations_recovery
    ON effect_operations(surface_id, prepared_at, id) WHERE phase <> 'finalized';
CREATE TABLE effect_operation_artifacts (
    id TEXT PRIMARY KEY, operation_id TEXT NOT NULL REFERENCES effect_operations(id),
    artifact_role TEXT NOT NULL, locator_kind TEXT NOT NULL, locator_key TEXT NOT NULL,
    locator_json TEXT NOT NULL, expected_fingerprint TEXT NOT NULL, state TEXT NOT NULL,
    created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL,
    UNIQUE (operation_id, artifact_role, locator_kind, locator_key), UNIQUE (locator_kind, locator_key),
    CHECK (updated_at >= created_at)
);
CREATE INDEX effect_operation_artifacts_cleanup
    ON effect_operation_artifacts(state, updated_at, id) WHERE state IN ('pending_cleanup', 'cleanup_failed');
CREATE TABLE effect_audit_events (
    id TEXT PRIMARY KEY, workspace_id TEXT, surface_id TEXT, source_id TEXT,
    subject_kind TEXT NOT NULL, subject_id TEXT NOT NULL, event_kind TEXT NOT NULL,
    generation INTEGER, initiator_kind TEXT NOT NULL, initiator_id TEXT,
    payload_version INTEGER NOT NULL, payload_json TEXT NOT NULL, occurred_at INTEGER NOT NULL,
    CHECK (generation IS NULL OR generation >= 0), CHECK (payload_version > 0)
);
CREATE INDEX effect_audit_events_workspace_time
    ON effect_audit_events(workspace_id, occurred_at, id) WHERE workspace_id IS NOT NULL;
CREATE INDEX effect_audit_events_surface_time
    ON effect_audit_events(surface_id, occurred_at, id) WHERE surface_id IS NOT NULL;
CREATE INDEX effect_audit_events_source_time
    ON effect_audit_events(source_id, occurred_at, id) WHERE source_id IS NOT NULL;
CREATE INDEX effect_audit_events_subject_time
    ON effect_audit_events(subject_kind, subject_id, occurred_at, id);
CREATE INDEX effect_audit_events_kind_time
    ON effect_audit_events(event_kind, occurred_at, id);
CREATE TRIGGER effect_audit_events_no_update BEFORE UPDATE ON effect_audit_events
BEGIN SELECT RAISE(ABORT, 'effect audit events are append-only'); END;

-- Every Workspace owns exactly one Effect aggregate. Desired defaults are synchronized by the
-- repository because generation and reconcile wakeups must be committed with the item rows.
CREATE TRIGGER workspace_effects_after_workspace_insert AFTER INSERT ON workspaces
BEGIN
    INSERT INTO workspace_effects (workspace_id, generation, created_at, updated_at)
    VALUES (NEW.id, 0, NEW.created_at, NEW.updated_at);
    INSERT INTO workspace_effect_desired_items (
        id, workspace_id, source_id, revision_id, created_at, updated_at
    )
    SELECT lower(hex(randomblob(16))), NEW.id, sources.id, heads.revision_id,
           NEW.created_at, NEW.created_at
    FROM effect_sources sources
    JOIN effect_source_heads heads ON heads.source_id = sources.id
    WHERE sources.effect_kind = 'skill' AND sources.lifecycle = 'active';
    UPDATE workspace_effects
    SET generation = CASE
            WHEN EXISTS (
                SELECT 1 FROM workspace_effect_desired_items WHERE workspace_id = NEW.id
            ) THEN 1 ELSE 0 END,
        updated_at = NEW.updated_at
    WHERE workspace_id = NEW.id;
END;
INSERT INTO workspace_effects (workspace_id, generation, created_at, updated_at)
SELECT id, 0, created_at, updated_at FROM workspaces;
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP TRIGGER IF EXISTS workspace_effects_after_workspace_insert;
DROP TRIGGER IF EXISTS effect_audit_events_no_update;
DROP TABLE IF EXISTS effect_audit_events;
DROP TABLE IF EXISTS effect_operation_artifacts;
DROP TABLE IF EXISTS effect_operations;
DROP TABLE IF EXISTS effect_propagation_requests;
DROP TABLE IF EXISTS effect_reconcile_requests;
DROP TABLE IF EXISTS effect_conditions;
DROP TABLE IF EXISTS effect_consumer_status;
DROP TABLE IF EXISTS effect_surface_status;
DROP TABLE IF EXISTS effect_managed_items;
DROP TABLE IF EXISTS workspace_effect_desired_items;
DROP TABLE IF EXISTS effect_surface_consumers;
DROP TABLE IF EXISTS effect_surfaces;
DROP TABLE IF EXISTS workspace_effects;
DROP TABLE IF EXISTS effect_source_heads;
DROP TABLE IF EXISTS effect_source_revisions;
DROP TABLE IF EXISTS effect_sources;
"#];

/// Builds the normalized Effect v2 persistence model.
pub fn migration() -> Migration {
    Migration::new("0006", UP_STATEMENTS, DOWN_STATEMENTS)
}
