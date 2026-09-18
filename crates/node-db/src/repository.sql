CREATE TABLE execution_identities (
    operation TEXT PRIMARY KEY,
    execution TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK(kind IN ('worktree','clone'))
);
INSERT INTO execution_identities SELECT operation,execution,'worktree' FROM executions;
CREATE TRIGGER register_worktree_execution AFTER INSERT ON executions BEGIN
    INSERT INTO execution_identities VALUES (NEW.operation,NEW.execution,'worktree');
END;
CREATE TABLE clone_executions (
    operation TEXT PRIMARY KEY REFERENCES execution_identities(operation),
    execution TEXT NOT NULL UNIQUE REFERENCES execution_identities(execution),
    input TEXT NOT NULL CHECK(json_valid(input)),
    repository TEXT NOT NULL UNIQUE,
    path TEXT NOT NULL UNIQUE,
    target TEXT NOT NULL CHECK(json_valid(target)),
    state TEXT NOT NULL CHECK(state IN ('accepted','running','unknown','completed')),
    progress TEXT NOT NULL CHECK(json_valid(progress)),
    CHECK(operation = json_extract(input, '$.operation_id')),
    CHECK(execution = json_extract(input, '$.execution_id'))
);
CREATE TABLE clone_outbox (
    execution TEXT PRIMARY KEY REFERENCES clone_executions(execution),
    event TEXT NOT NULL CHECK(json_valid(event))
);
ALTER TABLE process_attempts RENAME TO previous_process_attempts;
ALTER TABLE managed_executions RENAME TO previous_managed_executions;
CREATE TABLE managed_executions (
    execution TEXT PRIMARY KEY REFERENCES execution_identities(execution)
);
CREATE TABLE process_attempts (
    run TEXT PRIMARY KEY,
    execution TEXT NOT NULL REFERENCES managed_executions(execution),
    data BLOB NOT NULL,
    cleaned INTEGER NOT NULL CHECK(cleaned IN (0,1))
);
INSERT INTO managed_executions SELECT * FROM previous_managed_executions;
INSERT INTO process_attempts SELECT * FROM previous_process_attempts;
DROP TABLE previous_process_attempts;
DROP TABLE previous_managed_executions;
CREATE TABLE process_outcomes (
    run TEXT PRIMARY KEY REFERENCES process_attempts(run),
    exit_code INTEGER NOT NULL
);
