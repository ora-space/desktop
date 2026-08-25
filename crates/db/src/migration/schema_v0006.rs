use super::Migration;

const UP_STATEMENTS: &[&str] = &[r#"
CREATE TABLE effect_source_states (
    source_kind        TEXT NOT NULL CHECK (source_kind IN ('local', 'plugin')),
    namespace          TEXT NOT NULL COLLATE NOCASE,
    skill_name         TEXT NOT NULL COLLATE NOCASE,
    display_name       TEXT NOT NULL,
    source_version     TEXT NOT NULL,
    skill_md_digest    TEXT NOT NULL,
    package_root       TEXT NOT NULL,
    availability       TEXT NOT NULL CHECK (availability IN ('available', 'unavailable')),
    unavailable_reason TEXT,
    updated_at         INTEGER NOT NULL,
    PRIMARY KEY (source_kind, namespace, skill_name),
    CHECK (
        (availability = 'available' AND unavailable_reason IS NULL)
        OR (availability = 'unavailable' AND unavailable_reason IS NOT NULL)
    )
);

CREATE TABLE workspace_effects (
    workspace_id TEXT PRIMARY KEY REFERENCES workspaces(id),
    generation   INTEGER NOT NULL DEFAULT 0 CHECK (generation >= 0),
    updated_at   INTEGER NOT NULL
);

CREATE TABLE workspace_effect_desired_skills (
    workspace_id    TEXT NOT NULL REFERENCES workspace_effects(workspace_id) ON DELETE CASCADE,
    source_kind     TEXT NOT NULL CHECK (source_kind IN ('local', 'plugin')),
    namespace       TEXT NOT NULL COLLATE NOCASE,
    skill_name      TEXT NOT NULL COLLATE NOCASE,
    display_name    TEXT NOT NULL,
    source_version  TEXT NOT NULL,
    skill_md_digest TEXT NOT NULL,
    PRIMARY KEY (workspace_id, source_kind, namespace, skill_name)
);

CREATE INDEX idx_effect_desired_source_reference
    ON workspace_effect_desired_skills(source_kind, namespace, skill_name, source_version);

CREATE TABLE effect_surfaces (
    workspace_id           TEXT NOT NULL REFERENCES workspaces(id),
    surface_key            TEXT NOT NULL,
    workspace_path         TEXT NOT NULL,
    relative_path          TEXT NOT NULL,
    materialization_format TEXT NOT NULL,
    lifecycle              TEXT NOT NULL CHECK (lifecycle IN ('active', 'retiring')),
    consumers_json         TEXT NOT NULL,
    created_at             INTEGER NOT NULL,
    updated_at             INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, surface_key)
);

CREATE UNIQUE INDEX effect_surfaces_workspace_path_unique
    ON effect_surfaces(workspace_id, relative_path);

CREATE TABLE effect_managed_skills (
    managed_identity    TEXT PRIMARY KEY,
    workspace_id        TEXT NOT NULL REFERENCES workspaces(id),
    surface_key         TEXT NOT NULL,
    source_kind         TEXT NOT NULL CHECK (source_kind IN ('local', 'plugin')),
    namespace           TEXT NOT NULL COLLATE NOCASE,
    skill_name          TEXT NOT NULL COLLATE NOCASE,
    display_name        TEXT NOT NULL,
    source_version      TEXT NOT NULL,
    skill_md_digest     TEXT NOT NULL,
    locator             TEXT NOT NULL COLLATE NOCASE,
    target_name         TEXT NOT NULL,
    applied_fingerprint TEXT NOT NULL,
    applied_generation  INTEGER NOT NULL CHECK (applied_generation >= 0),
    FOREIGN KEY (workspace_id, surface_key)
        REFERENCES effect_surfaces(workspace_id, surface_key)
);

CREATE UNIQUE INDEX effect_managed_surface_selection_unique
    ON effect_managed_skills(workspace_id, surface_key, source_kind, namespace, skill_name);
CREATE UNIQUE INDEX effect_managed_surface_locator_unique
    ON effect_managed_skills(workspace_id, surface_key, locator);

CREATE TABLE effect_surface_status (
    workspace_id        TEXT NOT NULL REFERENCES workspaces(id),
    surface_key         TEXT NOT NULL,
    desired_generation  INTEGER NOT NULL CHECK (desired_generation >= 0),
    observed_generation INTEGER NOT NULL CHECK (observed_generation >= 0),
    applied_generation  INTEGER NOT NULL CHECK (applied_generation >= 0),
    phase               TEXT NOT NULL,
    revision            INTEGER NOT NULL CHECK (revision > 0),
    updated_at          INTEGER NOT NULL,
    conditions_json     TEXT NOT NULL,
    PRIMARY KEY (workspace_id, surface_key),
    FOREIGN KEY (workspace_id, surface_key)
        REFERENCES effect_surfaces(workspace_id, surface_key)
);

CREATE TABLE effect_consumer_status (
    surface_key      TEXT NOT NULL,
    consumer_id      TEXT NOT NULL,
    ready_generation INTEGER NOT NULL CHECK (ready_generation >= 0),
    phase            TEXT NOT NULL,
    revision         INTEGER NOT NULL CHECK (revision > 0),
    updated_at       INTEGER NOT NULL,
    conditions_json  TEXT NOT NULL,
    PRIMARY KEY (surface_key, consumer_id)
);

CREATE TABLE effect_operations (
    operation_id   TEXT PRIMARY KEY,
    workspace_id   TEXT NOT NULL REFERENCES workspaces(id),
    surface_key    TEXT NOT NULL,
    generation     INTEGER NOT NULL CHECK (generation >= 0),
    locator        TEXT NOT NULL,
    operation_kind TEXT NOT NULL CHECK (operation_kind IN ('create', 'update', 'replace', 'delete')),
    phase          TEXT NOT NULL CHECK (phase IN ('prepared', 'applied', 'finalized')),
    payload_json   TEXT NOT NULL,
    prepared_at    INTEGER NOT NULL DEFAULT (unixepoch('subsec') * 1000)
);

CREATE INDEX idx_effect_operations_recovery
    ON effect_operations(phase, prepared_at, operation_id);

CREATE TABLE effect_reconcile_requests (
    workspace_id         TEXT NOT NULL REFERENCES workspaces(id),
    surface_key          TEXT NOT NULL,
    requested_generation INTEGER NOT NULL CHECK (requested_generation >= 0),
    requested_at         INTEGER NOT NULL,
    PRIMARY KEY (workspace_id, surface_key)
);

CREATE TABLE effect_source_propagation_requests (
    source_kind       TEXT NOT NULL CHECK (source_kind IN ('local', 'plugin')),
    namespace         TEXT NOT NULL COLLATE NOCASE,
    skill_name        TEXT NOT NULL COLLATE NOCASE,
    requested_version TEXT NOT NULL,
    requested_at      INTEGER NOT NULL,
    PRIMARY KEY (source_kind, namespace, skill_name)
);

CREATE TABLE effect_audit_events (
    id           INTEGER PRIMARY KEY,
    workspace_id TEXT,
    surface_key  TEXT,
    event_kind   TEXT NOT NULL,
    generation   INTEGER,
    occurred_at  INTEGER NOT NULL
);
"#];

const DOWN_STATEMENTS: &[&str] = &[r#"
DROP TABLE IF EXISTS effect_audit_events;
DROP TABLE IF EXISTS effect_source_propagation_requests;
DROP TABLE IF EXISTS effect_reconcile_requests;
DROP TABLE IF EXISTS effect_operations;
DROP TABLE IF EXISTS effect_consumer_status;
DROP TABLE IF EXISTS effect_surface_status;
DROP TABLE IF EXISTS effect_managed_skills;
DROP TABLE IF EXISTS effect_surfaces;
DROP TABLE IF EXISTS workspace_effect_desired_skills;
DROP TABLE IF EXISTS workspace_effects;
DROP TABLE IF EXISTS effect_source_states;
"#];

/// Adds the first Workspace Effect Skill State persistence boundary.
pub fn migration() -> Migration {
    Migration::new("0006", UP_STATEMENTS, DOWN_STATEMENTS)
}
