DROP INDEX queued_session_commands;
ALTER TABLE session_commands RENAME TO previous_session_commands;
CREATE TABLE session_commands (
    acceptance_order INTEGER PRIMARY KEY AUTOINCREMENT,
    execution TEXT NOT NULL REFERENCES node_executions(execution),
    command_id TEXT NOT NULL CHECK(length(command_id)>0),
    input TEXT NOT NULL CHECK(json_valid(input)),
    state TEXT NOT NULL CHECK(state IN ('queued','executed','discarded','failed')),
    UNIQUE(execution,command_id)
);
INSERT INTO session_commands SELECT * FROM previous_session_commands;
DROP TABLE previous_session_commands;
CREATE INDEX queued_session_commands ON session_commands(execution,state,acceptance_order);
CREATE TABLE session_turn_failures (
    execution TEXT NOT NULL REFERENCES node_executions(execution),
    turn_id TEXT NOT NULL,
    failure TEXT NOT NULL CHECK(json_valid(failure)),
    sequence INTEGER NOT NULL,
    PRIMARY KEY(execution,turn_id)
);
CREATE TABLE session_stop_attempts (
    attempt TEXT PRIMARY KEY,
    execution TEXT NOT NULL REFERENCES node_executions(execution),
    checkout TEXT NOT NULL,
    record TEXT NOT NULL CHECK(json_valid(record)),
    state TEXT NOT NULL CHECK(state IN ('prepared','stopping','stop_unconfirmed','legacy_unknown','confirmed')),
    created_at_ms INTEGER NOT NULL,
    first_stop_at_ms INTEGER,
    bound_at_ms INTEGER,
    confirmed_at_ms INTEGER,
    evidence TEXT CHECK(evidence IS NULL OR evidence IN ('lock_and_processes','host_reboot','fenced_volume')),
    resources_released INTEGER NOT NULL DEFAULT 0 CHECK(resources_released IN (0,1)),
    CHECK((first_stop_at_ms IS NULL) = (bound_at_ms IS NULL)),
    CHECK(bound_at_ms IS NULL OR bound_at_ms > first_stop_at_ms),
    CHECK((state='confirmed') = (confirmed_at_ms IS NOT NULL)),
    CHECK(resources_released=0 OR state='confirmed')
);
CREATE INDEX session_stop_execution ON session_stop_attempts(execution,state);
CREATE INDEX session_stop_checkout ON session_stop_attempts(checkout,state);
CREATE TABLE session_stop_summary (
    execution TEXT PRIMARY KEY REFERENCES node_executions(execution),
    confirmed_attempts INTEGER NOT NULL,
    last_confirmed_at_ms INTEGER NOT NULL
);
CREATE TABLE session_stop_ends (
    execution TEXT PRIMARY KEY REFERENCES node_executions(execution),
    ended TEXT NOT NULL CHECK(json_valid(ended))
);
CREATE TABLE session_stop_audit (
    request_id TEXT PRIMARY KEY,
    execution TEXT NOT NULL,
    attempt TEXT NOT NULL,
    actor_uid INTEGER NOT NULL CHECK(actor_uid=0),
    request TEXT NOT NULL CHECK(json_valid(request)),
    requested_at_ms INTEGER NOT NULL,
    result TEXT CHECK(result IS NULL OR json_valid(result)),
    completed_at_ms INTEGER,
    CHECK((result IS NULL) = (completed_at_ms IS NULL))
);
