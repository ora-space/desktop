DROP TRIGGER workflow_debug_admit;
DROP TRIGGER workflow_debug_pack;

-- SQLite serializes writers. Each enclosing trigger consumes these small references
-- before it returns; a statement abort rolls back the buffer along with the attempt.
CREATE TABLE workflow_debug_pending_refs (owner TEXT, key TEXT, outcome_id INTEGER);
CREATE VIEW workflow_debug_value_input(run_id,owner,key,outcome,digest) AS
    SELECT NULL,NULL,NULL,NULL,NULL WHERE false;

CREATE TRIGGER workflow_debug_intern INSTEAD OF INSERT ON workflow_debug_value_input
BEGIN
    -- Avoid constructing a large SQLite record for an existing value. On a digest
    -- collision the insert reaches the existing byte-comparison trigger and aborts.
    INSERT INTO workflow_debug_outcomes(run_id,outcome,digest)
    SELECT NEW.run_id,NEW.outcome,NEW.digest WHERE NEW.outcome IS NOT NULL
        AND NOT EXISTS (SELECT 1 FROM workflow_debug_outcomes
            WHERE run_id=NEW.run_id AND digest=NEW.digest AND outcome=NEW.outcome);
    -- A NULL outcome leaves an empty ledger owner visible to the final grouping.
    INSERT INTO workflow_debug_pending_refs(owner,key,outcome_id)
    SELECT NEW.owner,NEW.key,(SELECT id FROM workflow_debug_outcomes
        WHERE run_id=NEW.run_id AND digest=NEW.digest)
    WHERE NEW.outcome IS NOT NULL OR NEW.owner IS NOT NULL;
END;

CREATE TRIGGER workflow_debug_pack INSTEAD OF INSERT ON workflow_debug_document_input
BEGIN
    INSERT INTO workflow_debug_value_input(run_id,owner,key,outcome,digest)
    WITH pool(body) AS MATERIALIZED (
        SELECT jsonb(NEW.body) WHERE json_valid(NEW.body)
    ), entries(key,outcome) AS MATERIALIZED (
        SELECT v.key,p.body -> v.fullkey FROM pool p CROSS JOIN json_each(p.body,'$.variablePool.values') v
        WHERE json_type(p.body,'$.variablePool.values')='object'
    )
    SELECT NEW.run_id,NULL,key,outcome,ora_debug_hash(outcome) FROM entries;

    INSERT INTO workflow_debug_documents(node_run_id,kind,body,refs)
    SELECT NEW.node_run_id,NEW.kind,
        CASE WHEN t='object' THEN json_remove(NEW.body,'$.variablePool.values') ELSE NEW.body END,
        CASE WHEN t='object' THEN (SELECT json_group_object(key,outcome_id) FROM workflow_debug_pending_refs) END
    FROM (SELECT json_type(CASE WHEN json_valid(NEW.body) THEN NEW.body END,'$.variablePool.values') t);
    DELETE FROM workflow_debug_pending_refs;
END;

CREATE TRIGGER workflow_debug_admit AFTER INSERT ON workflow_node_runs
BEGIN
    INSERT INTO workflow_debug_document_input(node_run_id,run_id,kind,body)
    SELECT NEW.id,r.id,'run',
        CASE WHEN json_type(CASE WHEN json_valid(r.payload) THEN r.payload END,'$.iterationLedger')='object'
             THEN json_remove(r.payload,'$.iterationLedger') ELSE r.payload END
    FROM workflow_runs r WHERE r.id=NEW.run_id AND NEW.node_type IN ('agent','condition','loop','iteration')
    UNION ALL SELECT NEW.id,NEW.run_id,'scope',state FROM workflow_execution_scopes
    WHERE id=NEW.scope_id AND state IS NOT NULL AND NEW.node_type IN ('agent','condition','loop','iteration');
    -- Pack the run/scope documents first: each document consumes its own buffered refs.
    INSERT INTO workflow_debug_value_input(run_id,owner,key,outcome,digest)
    WITH ledger(owner,key,outcome) AS MATERIALIZED (
        SELECT owner.key,item.key,item.value FROM workflow_runs r
        CROSS JOIN json_each(CASE WHEN json_valid(r.payload) THEN r.payload END,'$.iterationLedger') owner
        LEFT JOIN json_each(owner.value) item ON true
        WHERE r.id=NEW.run_id AND NEW.node_type IN ('agent','condition','loop','iteration')
    )
    SELECT NEW.run_id,owner,key,outcome,CASE WHEN outcome IS NOT NULL THEN ora_debug_hash(outcome) END
    FROM ledger
    UNION ALL
    SELECT id,NULL,NULL,input,ora_debug_hash(input) FROM workflow_runs
    WHERE id=NEW.run_id AND input IS NOT NULL AND NEW.node_type IN ('agent','condition','loop','iteration');
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
               json_object('inputRef',(SELECT outcome_id FROM workflow_debug_pending_refs WHERE owner IS NULL),
                   'ledgerRefs', CASE WHEN json_type(CASE WHEN json_valid(r.payload) THEN r.payload END,'$.iterationLedger')='object'
                       THEN (SELECT json_group_object(owner,json(refs)) FROM (
                           SELECT owner,json_group_object(key,outcome_id) FILTER (WHERE outcome_id IS NOT NULL) AS refs
                           FROM workflow_debug_pending_refs WHERE owner IS NOT NULL GROUP BY owner
                       )) END,
                       'scopeId', NEW.scope_id, 'round', s.round_index, 'index', NEW.iteration,
                       'completedAttempts', (SELECT json_group_array(id) FROM workflow_node_runs
                            WHERE run_id = r.id AND scope_id = NEW.scope_id AND is_deleted = 0 AND status = 2)) END
    FROM workflow_runs r JOIN workflow_snapshots snap ON snap.id = r.snapshot_id
    LEFT JOIN workflow_execution_scopes s ON s.id = NEW.scope_id
    WHERE r.id = NEW.run_id;
    DELETE FROM workflow_debug_pending_refs;
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
