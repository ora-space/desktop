-- Monotonic writer versions distinguish replacement attempts and change-then-restore races.
CREATE TABLE workflow_debug_node_versions (
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    node_id TEXT NOT NULL,
    version INTEGER NOT NULL,
    PRIMARY KEY(run_id,node_id)
);
CREATE TRIGGER workflow_debug_node_insert AFTER INSERT ON workflow_node_runs
BEGIN
    INSERT INTO workflow_debug_node_versions VALUES(NEW.run_id,NEW.node_id,1)
        ON CONFLICT(run_id,node_id) DO UPDATE SET version=version+1;
END;
CREATE TRIGGER workflow_debug_node_change AFTER UPDATE ON workflow_node_runs
BEGIN
    INSERT INTO workflow_debug_node_versions VALUES(NEW.run_id,NEW.node_id,1)
        ON CONFLICT(run_id,node_id) DO UPDATE SET version=version+1;
END;
CREATE TABLE workflow_debug_attempts (
    node_run_id TEXT PRIMARY KEY REFERENCES workflow_node_runs(id) ON DELETE CASCADE,
    snapshot_id TEXT NOT NULL REFERENCES workflow_snapshots(id),
    revision INTEGER NOT NULL,
    container_attempt TEXT REFERENCES workflow_node_runs(id),
    entry_state TEXT,
    dispatch_input TEXT
);

CREATE TRIGGER workflow_debug_dispatch_recorded AFTER UPDATE OF dispatch_input ON workflow_debug_attempts
BEGIN
    UPDATE workflow_debug_node_versions SET version=version+1 WHERE (run_id,node_id)=(
        SELECT run_id,node_id FROM workflow_node_runs WHERE id=NEW.node_run_id);
END;
CREATE TRIGGER workflow_debug_node_removed AFTER DELETE ON workflow_node_runs
BEGIN
    UPDATE workflow_debug_node_versions SET version=version+1 WHERE run_id=OLD.run_id AND node_id=OLD.node_id;
END;

CREATE TABLE workflow_debug_previews (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    snapshot_id TEXT NOT NULL REFERENCES workflow_snapshots(id),
    root_scope TEXT NOT NULL REFERENCES workflow_execution_scopes(id),
    node_type TEXT NOT NULL,
    read_nodes TEXT NOT NULL,
    read_facts TEXT NOT NULL,
    base_revision INTEGER NOT NULL,
    patch TEXT NOT NULL,
    plan TEXT NOT NULL,
    UNIQUE(run_id, id)
);

CREATE TABLE workflow_debug_operations (
    id INTEGER PRIMARY KEY,
    run_id TEXT NOT NULL,
    request_id TEXT NOT NULL,
    preview_id TEXT NOT NULL UNIQUE,
    state TEXT NOT NULL CHECK(state IN ('stopping','stop_failed','applied')),
    revision INTEGER,
    failure TEXT,
    -- JSON array of the attempts a failed stop could not prove settled.
    unsettled_attempts TEXT NOT NULL DEFAULT '[]',
    -- The run as it was when the confirmation was accepted. A terminal run returns to it when
    -- the debugged scope does not settle successfully.
    prior_status INTEGER NOT NULL,
    prior_error TEXT,
    prior_output TEXT,
    prior_finished_at INTEGER,
    released INTEGER NOT NULL DEFAULT 0 CHECK(released IN (0,1)),
    CHECK ((state = 'applied') = (revision IS NOT NULL)),
    CHECK ((state = 'stop_failed') = (failure IS NOT NULL)),
    CHECK (state = 'stop_failed' OR unsettled_attempts = '[]'),
    UNIQUE(run_id, request_id),
    UNIQUE(run_id, revision),
    FOREIGN KEY(run_id, preview_id) REFERENCES workflow_debug_previews(run_id, id) ON DELETE CASCADE
);
CREATE UNIQUE INDEX workflow_debug_one_stopping ON workflow_debug_operations(run_id)
    WHERE state = 'stopping' OR (state = 'stop_failed' AND released = 0);

CREATE TABLE workflow_debug_retired_attempts (
    node_run_id TEXT PRIMARY KEY REFERENCES workflow_node_runs(id) ON DELETE CASCADE,
    operation_id INTEGER NOT NULL REFERENCES workflow_debug_operations(id) ON DELETE CASCADE
);
-- The attempt the scheduler first admitted for a confirmed, still undispatched position.
CREATE TABLE workflow_debug_first_attempts (
    operation_id INTEGER PRIMARY KEY REFERENCES workflow_debug_operations(id) ON DELETE CASCADE,
    node_run_id TEXT NOT NULL REFERENCES workflow_node_runs(id) ON DELETE CASCADE
);
-- A successful snapshot switch starts a new configuration baseline, also when the run returns
-- to a snapshot it used before. Earlier revisions remain readable as history and are not replayed.
CREATE TABLE workflow_debug_baselines (
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    revision INTEGER NOT NULL,
    snapshot_id TEXT NOT NULL REFERENCES workflow_snapshots(id),
    PRIMARY KEY(run_id,revision)
);
CREATE VIEW workflow_debug_revisions AS
    SELECT run_id,revision FROM workflow_debug_baselines
    UNION ALL SELECT run_id,revision FROM workflow_debug_operations WHERE revision IS NOT NULL;

-- Record only new attempts. A retry waiting for its backoff has no dispatch evidence yet.
CREATE TRIGGER workflow_debug_admit AFTER INSERT ON workflow_node_runs
BEGIN
    INSERT INTO workflow_debug_attempts(node_run_id, snapshot_id, revision, container_attempt, entry_state)
    SELECT NEW.id, r.snapshot_id,
           COALESCE((SELECT MAX(revision) FROM workflow_debug_revisions WHERE run_id = r.id), 0),
           COALESCE(s.parent_loop_node_run_id, (
               SELECT owner.id FROM json_each(CASE WHEN json_valid(snap.graph) THEN snap.graph END, '$.nodes') n
               JOIN workflow_node_runs owner ON owner.run_id = r.id AND owner.scope_id = NEW.scope_id
                   AND owner.node_id = COALESCE(json_extract(n.value,'$.parentId'), json_extract(n.value,'$.data.containerId'))
                   AND owner.node_type = 'iteration' AND owner.is_deleted = 0
               WHERE NEW.iteration IS NOT NULL AND json_extract(n.value,'$.id') = NEW.node_id
           )),
           CASE WHEN NEW.node_type IN ('agent','condition','loop','iteration') THEN
               json_object('runInput', r.input, 'runPayload', r.payload, 'scopeState', s.state,
                       'scopeId', NEW.scope_id, 'round', s.round_index, 'index', NEW.iteration,
                       'completedAttempts', (SELECT json_group_array(id) FROM workflow_node_runs
                            WHERE run_id = r.id AND scope_id = NEW.scope_id AND is_deleted = 0 AND status = 2)) END
    FROM workflow_runs r JOIN workflow_snapshots snap ON snap.id = r.snapshot_id
    LEFT JOIN workflow_execution_scopes s ON s.id = NEW.scope_id
    WHERE r.id = NEW.run_id;
    -- Bind the first attempt of a confirmed logical position to the operation that scheduled it.
    INSERT INTO workflow_debug_first_attempts(operation_id,node_run_id)
    SELECT op.id,NEW.id FROM workflow_debug_operations op
    JOIN workflow_debug_previews p ON p.id=op.preview_id
    JOIN workflow_debug_attempts a ON a.node_run_id=NEW.id AND a.snapshot_id=p.snapshot_id
    WHERE op.run_id=NEW.run_id
      AND op.revision>COALESCE((SELECT MAX(revision) FROM workflow_debug_baselines WHERE run_id=op.run_id),0)
      AND json_extract(p.plan,'$.target.nodeRunId') IS NULL
      AND json_extract(p.plan,'$.target.scopeId')=NEW.scope_id
      AND json_extract(p.plan,'$.target.nodeId')=NEW.node_id
      AND json_extract(p.plan,'$.target.containerNodeRunId') IS a.container_attempt
      AND json_extract(p.plan,'$.target.index') IS NEW.iteration
      AND NOT EXISTS(SELECT 1 FROM workflow_debug_first_attempts bound
          WHERE bound.operation_id=op.id);
END;
