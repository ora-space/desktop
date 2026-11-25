CREATE TABLE session_turn_failure_settlements (
    execution TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    outcome TEXT NOT NULL CHECK(outcome IN ('kept','session_ending')),
    sequence INTEGER NOT NULL,
    PRIMARY KEY (execution, turn_id)
);