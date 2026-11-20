DROP INDEX session_stop_execution;
DROP INDEX session_stop_checkout;
ALTER TABLE session_stop_attempts RENAME TO previous_session_stop_attempts;
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
    evidence TEXT CHECK(evidence IS NULL OR evidence IN ('lock_and_processes','lock_processes_and_marks','host_reboot','fenced_volume')),
    resources_released INTEGER NOT NULL DEFAULT 0 CHECK(resources_released IN (0,1)),
    CHECK((first_stop_at_ms IS NULL) = (bound_at_ms IS NULL)),
    CHECK(bound_at_ms IS NULL OR bound_at_ms > first_stop_at_ms),
    CHECK((state='confirmed') = (confirmed_at_ms IS NOT NULL)),
    CHECK(resources_released=0 OR state='confirmed')
);
INSERT INTO session_stop_attempts SELECT * FROM previous_session_stop_attempts;
DROP TABLE previous_session_stop_attempts;
CREATE INDEX session_stop_execution ON session_stop_attempts(execution,state);
CREATE INDEX session_stop_checkout ON session_stop_attempts(checkout,state);
