ALTER TABLE workflow_runs ADD COLUMN debug_epoch INTEGER NOT NULL DEFAULT 0;
CREATE TRIGGER workflow_debug_run_fact AFTER UPDATE OF run_status, snapshot_id, state, input, payload, is_deleted ON workflow_runs
BEGIN UPDATE workflow_runs SET debug_epoch = debug_epoch + 1 WHERE id = NEW.id; END;
CREATE TRIGGER workflow_debug_node_insert AFTER INSERT ON workflow_node_runs
BEGIN UPDATE workflow_runs SET debug_epoch = debug_epoch + 1 WHERE id = NEW.run_id; END;
CREATE TRIGGER workflow_debug_node_change AFTER UPDATE ON workflow_node_runs
BEGIN UPDATE workflow_runs SET debug_epoch = debug_epoch + 1 WHERE id = NEW.run_id; END;
CREATE TRIGGER workflow_debug_scope_insert AFTER INSERT ON workflow_execution_scopes
BEGIN UPDATE workflow_runs SET debug_epoch = debug_epoch + 1 WHERE id = NEW.run_id; END;
CREATE TRIGGER workflow_debug_scope_change AFTER UPDATE ON workflow_execution_scopes
BEGIN UPDATE workflow_runs SET debug_epoch = debug_epoch + 1 WHERE id = NEW.run_id; END;
CREATE TRIGGER workflow_debug_root_change AFTER UPDATE ON workflow_run_root_scopes
BEGIN UPDATE workflow_runs SET debug_epoch = debug_epoch + 1 WHERE id = NEW.run_id; END;

CREATE TABLE workflow_debug_attempts (
    node_run_id TEXT PRIMARY KEY REFERENCES workflow_node_runs(id) ON DELETE CASCADE,
    snapshot_id TEXT NOT NULL REFERENCES workflow_snapshots(id),
    revision INTEGER NOT NULL,
    container_attempt TEXT REFERENCES workflow_node_runs(id),
    entry_state TEXT,
    dispatch_input TEXT
);

CREATE TABLE workflow_debug_previews (
    id TEXT PRIMARY KEY,
    run_id TEXT NOT NULL REFERENCES workflow_runs(id) ON DELETE CASCADE,
    target TEXT NOT NULL REFERENCES workflow_node_runs(id),
    base_revision INTEGER NOT NULL,
    patch TEXT NOT NULL,
    plan TEXT NOT NULL,
    epoch INTEGER NOT NULL,
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
    released INTEGER NOT NULL DEFAULT 0 CHECK(released IN (0,1)),
    CHECK ((state = 'applied') = (revision IS NOT NULL)),
    CHECK ((state = 'stop_failed') = (failure IS NOT NULL)),
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

-- Record only new attempts. A retry waiting for its backoff has no dispatch evidence yet.
CREATE TRIGGER workflow_debug_admit AFTER INSERT ON workflow_node_runs
BEGIN
    INSERT INTO workflow_debug_attempts(node_run_id, snapshot_id, revision, container_attempt, entry_state)
    SELECT NEW.id, r.snapshot_id,
           COALESCE((SELECT MAX(revision) FROM workflow_debug_operations WHERE run_id = r.id), 0),
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
END;
