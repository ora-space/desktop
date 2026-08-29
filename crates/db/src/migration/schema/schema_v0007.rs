use super::Migration;

const UP_STATEMENTS: &[&str] = &[
    r#"
-- Agent Target rows are the Expand-phase owner for future Skill+MCP convergence. Surface-keyed
-- Skill tables remain authoritative for the live worker until a later Contract ticket.
CREATE TABLE effect_agent_targets (
    id TEXT PRIMARY KEY,
    workspace_id TEXT NOT NULL REFERENCES workspace_effects(workspace_id) ON DELETE CASCADE,
    agent_plugin_id TEXT NOT NULL,
    capability_revision TEXT NOT NULL,
    lifecycle TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    UNIQUE (workspace_id, agent_plugin_id),
    CHECK (lifecycle IN ('active', 'retired')),
    CHECK (updated_at >= created_at)
);
CREATE INDEX effect_agent_targets_workspace
    ON effect_agent_targets(workspace_id, agent_plugin_id);
"#,
    r#"
CREATE TABLE effect_agent_target_status (
    agent_target_id TEXT PRIMARY KEY REFERENCES effect_agent_targets(id) ON DELETE CASCADE,
    desired_generation INTEGER NOT NULL DEFAULT 0,
    observed_generation INTEGER NOT NULL DEFAULT 0,
    applied_generation INTEGER NOT NULL DEFAULT 0,
    ready_generation INTEGER NOT NULL DEFAULT 0,
    phase TEXT NOT NULL,
    status_version INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (desired_generation >= 0),
    CHECK (observed_generation >= 0),
    CHECK (applied_generation >= 0),
    CHECK (ready_generation >= 0),
    CHECK (applied_generation <= observed_generation),
    CHECK (observed_generation <= desired_generation),
    CHECK (ready_generation <= applied_generation),
    CHECK (status_version > 0),
    CHECK (phase IN (
        'pending', 'waiting_for_idle', 'quiescing', 'applying', 'resuming',
        'current', 'ready_with_issues', 'degraded', 'retiring', 'recovery_required'
    )),
    CHECK (updated_at >= created_at)
);
"#,
    r#"
-- Parallel to surface-keyed effect_reconcile_requests. Nothing in production claims this table yet.
CREATE TABLE effect_agent_target_reconcile_requests (
    agent_target_id TEXT PRIMARY KEY REFERENCES effect_agent_targets(id) ON DELETE CASCADE,
    requested_generation INTEGER NOT NULL,
    request_token TEXT NOT NULL,
    state TEXT NOT NULL DEFAULT 'pending',
    wake_reason TEXT NOT NULL DEFAULT 'desired_changed',
    blocked_reason TEXT,
    lease_owner TEXT,
    lease_expires_at INTEGER,
    attempt_count INTEGER NOT NULL DEFAULT 0,
    requested_at INTEGER NOT NULL,
    not_before_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    CHECK (requested_generation >= 0),
    CHECK (attempt_count >= 0),
    CHECK (not_before_at >= requested_at),
    CHECK (updated_at >= requested_at),
    CHECK (state IN ('pending', 'claimed', 'blocked', 'retry_scheduled')),
    CHECK ((state = 'claimed') = (lease_owner IS NOT NULL)),
    CHECK ((state = 'claimed') = (lease_expires_at IS NOT NULL)),
    CHECK ((state = 'blocked') = (blocked_reason IS NOT NULL)),
    CHECK (wake_reason IN (
        'desired_changed', 'capability_changed', 'retry', 'recovery', 'startup_repair'
    ))
);
CREATE INDEX effect_agent_target_reconcile_requests_due
    ON effect_agent_target_reconcile_requests(state, not_before_at, requested_at, agent_target_id);
CREATE INDEX effect_agent_target_reconcile_requests_leases
    ON effect_agent_target_reconcile_requests(lease_expires_at) WHERE state = 'claimed';
"#,
    r#"
CREATE TABLE effect_agent_target_conditions (
    id TEXT PRIMARY KEY,
    agent_target_id TEXT NOT NULL
        REFERENCES effect_agent_target_status(agent_target_id) ON DELETE CASCADE,
    surface_id TEXT REFERENCES effect_surfaces(id) ON DELETE SET NULL,
    consumer_id TEXT,
    subject_kind TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    reason TEXT NOT NULL,
    impact TEXT NOT NULL,
    failed_generation INTEGER,
    message TEXT NOT NULL,
    first_observed_at INTEGER NOT NULL,
    last_observed_at INTEGER NOT NULL,
    CHECK (impact IN ('blocking', 'non_blocking')),
    CHECK (subject_kind IN (
        'agent_target', 'surface', 'consumer', 'desired_item', 'managed_item', 'mcp'
    )),
    CHECK (reason IN (
        'no_consumers', 'incompatible_surface_declarations', 'desired_collision',
        'preserved_conflict', 'ownership_conflict', 'drift_conflict', 'source_unavailable',
        'path_unsafe', 'scan_failed', 'waiting_for_idle', 'consumer_resume_failed',
        'materialization_failed', 'transient_io', 'recovery_required',
        'unsupported_by_agent', 'capability_invalid'
    )),
    CHECK (failed_generation IS NULL OR failed_generation >= 0),
    CHECK (last_observed_at >= first_observed_at),
    CHECK (consumer_id IS NULL OR surface_id IS NOT NULL),
    FOREIGN KEY (surface_id, consumer_id)
        REFERENCES effect_surface_consumers(surface_id, consumer_id) ON DELETE SET NULL
);
CREATE UNIQUE INDEX effect_agent_target_conditions_unique
    ON effect_agent_target_conditions(
        agent_target_id,
        subject_kind,
        subject_id,
        reason,
        IFNULL(surface_id, ''),
        IFNULL(consumer_id, '')
    );
"#,
    r#"
-- One Agent Target per historical surface consumer declaration.
INSERT INTO effect_agent_targets (
    id, workspace_id, agent_plugin_id, capability_revision, lifecycle, created_at, updated_at
)
SELECT lower(hex(randomblob(16))),
       surfaces.workspace_id,
       consumers.consumer_id,
       '',
       'active',
       MIN(consumers.created_at),
       MAX(consumers.updated_at)
FROM effect_surface_consumers consumers
JOIN effect_surfaces surfaces ON surfaces.id = consumers.surface_id
GROUP BY surfaces.workspace_id, consumers.consumer_id;
"#,
    r#"
-- Clamp independently maximized generations so Expand rows always satisfy ordering CHECKs.
-- Current is the caught-up state; a target whose ready generation lags desired stays Pending.
INSERT INTO effect_agent_target_status (
    agent_target_id, desired_generation, observed_generation, applied_generation,
    ready_generation, phase, status_version, created_at, updated_at
)
SELECT
    agent_target_id,
    desired_generation,
    observed_generation,
    applied_generation,
    ready_generation,
    CASE
        WHEN ready_generation = desired_generation THEN 'current'
        ELSE 'pending'
    END,
    1,
    created_at,
    updated_at
FROM (
    SELECT
        targets.id AS agent_target_id,
        MAX(COALESCE(surface_status.desired_generation, 0)) AS desired_generation,
        MIN(
            MAX(COALESCE(surface_status.desired_generation, 0)),
            MAX(COALESCE(surface_status.observed_generation, 0))
        ) AS observed_generation,
        MIN(
            MIN(
                MAX(COALESCE(surface_status.desired_generation, 0)),
                MAX(COALESCE(surface_status.observed_generation, 0))
            ),
            MAX(COALESCE(surface_status.applied_generation, 0))
        ) AS applied_generation,
        MIN(
            MIN(
                MIN(
                    MAX(COALESCE(surface_status.desired_generation, 0)),
                    MAX(COALESCE(surface_status.observed_generation, 0))
                ),
                MAX(COALESCE(surface_status.applied_generation, 0))
            ),
            MAX(COALESCE(consumer_status.ready_generation, 0))
        ) AS ready_generation,
        targets.created_at AS created_at,
        targets.updated_at AS updated_at
    FROM effect_agent_targets targets
    JOIN effect_surfaces surfaces
        ON surfaces.workspace_id = targets.workspace_id
    JOIN effect_surface_consumers consumers
        ON consumers.surface_id = surfaces.id
       AND consumers.consumer_id = targets.agent_plugin_id
    LEFT JOIN effect_surface_status surface_status
        ON surface_status.surface_id = surfaces.id
    LEFT JOIN effect_consumer_status consumer_status
        ON consumer_status.surface_id = surfaces.id
       AND consumer_status.consumer_id = consumers.consumer_id
    GROUP BY targets.id
);
"#,
    r#"
-- Merge every surface request that touches a target into one pending target request.
INSERT INTO effect_agent_target_reconcile_requests (
    agent_target_id, requested_generation, request_token, state, wake_reason,
    blocked_reason, lease_owner, lease_expires_at, attempt_count,
    requested_at, not_before_at, updated_at
)
SELECT
    targets.id,
    MAX(requests.requested_generation),
    lower(hex(randomblob(16))),
    'pending',
    'desired_changed',
    NULL,
    NULL,
    NULL,
    0,
    MIN(requests.requested_at),
    MAX(MIN(requests.not_before_at), MIN(requests.requested_at)),
    MAX(requests.updated_at)
FROM effect_reconcile_requests requests
JOIN effect_surfaces surfaces ON surfaces.id = requests.surface_id
JOIN effect_surface_consumers consumers ON consumers.surface_id = surfaces.id
JOIN effect_agent_targets targets
    ON targets.workspace_id = surfaces.workspace_id
   AND targets.agent_plugin_id = consumers.consumer_id
GROUP BY targets.id;
"#,
    r#"
-- Consumer-scoped surface conditions become Blocking target conditions on that plugin.
-- Preserve the stored subject so desired_item / managed_item identities survive upgrade.
INSERT INTO effect_agent_target_conditions (
    id, agent_target_id, surface_id, consumer_id, subject_kind, subject_id, reason, impact,
    failed_generation, message, first_observed_at, last_observed_at
)
SELECT lower(hex(randomblob(16))),
       targets.id,
       conditions.surface_id,
       conditions.consumer_id,
       conditions.subject_kind,
       conditions.subject_id,
       conditions.reason,
       'blocking',
       conditions.failed_generation,
       conditions.message,
       conditions.first_observed_at,
       conditions.last_observed_at
FROM effect_conditions conditions
JOIN effect_surfaces surfaces ON surfaces.id = conditions.surface_id
JOIN effect_agent_targets targets
    ON targets.workspace_id = surfaces.workspace_id
   AND targets.agent_plugin_id = conditions.consumer_id
WHERE conditions.consumer_id IS NOT NULL;
"#,
    r#"
-- Surface-scoped conditions fan out to every Agent Target that consumes the surface.
INSERT INTO effect_agent_target_conditions (
    id, agent_target_id, surface_id, consumer_id, subject_kind, subject_id, reason, impact,
    failed_generation, message, first_observed_at, last_observed_at
)
SELECT lower(hex(randomblob(16))),
       targets.id,
       conditions.surface_id,
       consumers.consumer_id,
       conditions.subject_kind,
       conditions.subject_id,
       conditions.reason,
       'blocking',
       conditions.failed_generation,
       conditions.message,
       conditions.first_observed_at,
       conditions.last_observed_at
FROM effect_conditions conditions
JOIN effect_surface_consumers consumers ON consumers.surface_id = conditions.surface_id
JOIN effect_surfaces surfaces ON surfaces.id = conditions.surface_id
JOIN effect_agent_targets targets
    ON targets.workspace_id = surfaces.workspace_id
   AND targets.agent_plugin_id = consumers.consumer_id
WHERE conditions.consumer_id IS NULL;
"#,
];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP TABLE IF EXISTS effect_agent_target_conditions;
DROP TABLE IF EXISTS effect_agent_target_reconcile_requests;
DROP TABLE IF EXISTS effect_agent_target_status;
DROP TABLE IF EXISTS effect_agent_targets;
"#];

/// Builds Agent Target Effect persistence beside the surface-keyed Skill model.
pub fn migration() -> Migration {
    Migration::new("0007", UP_STATEMENTS, DOWN_STATEMENTS)
}
