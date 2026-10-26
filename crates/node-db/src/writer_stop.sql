-- A recovered terminal is not proof that a previous plugin process tree stopped.
CREATE TABLE session_writer_stops (
    execution TEXT PRIMARY KEY REFERENCES node_executions(execution)
);
-- No backfill: legacy terminal rows may have been manufactured during restart recovery.
