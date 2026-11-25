CREATE VIEW workflow_debug_boot_protected AS
WITH stopped AS (
    SELECT o.run_id, x.value AS id FROM workflow_debug_operations o
    JOIN workflow_debug_previews p ON p.id=o.preview_id JOIN json_each(p.plan,'$.stop') x
    WHERE o.state='stopping' OR (o.state='stop_failed' AND o.released=0)
)
SELECT run_id,id FROM stopped UNION
SELECT s.run_id,a.container_attempt FROM stopped s JOIN workflow_debug_attempts a ON a.node_run_id=s.id
WHERE a.container_attempt IS NOT NULL;

-- The applied receipt is history. This pointer is the live dispatch authority and ends with the
-- run, so a later ordinary restart cannot inherit an obsolete restriction.
CREATE TABLE workflow_debug_dispatch (
    run_id TEXT PRIMARY KEY REFERENCES workflow_runs(id) ON DELETE CASCADE,
    operation_id INTEGER NOT NULL REFERENCES workflow_debug_operations(id) ON DELETE CASCADE
);
CREATE TRIGGER workflow_debug_end_dispatch AFTER UPDATE OF run_status ON workflow_runs
WHEN NEW.run_status <> 1 BEGIN DELETE FROM workflow_debug_dispatch WHERE run_id=NEW.id; END;
