CREATE TABLE session_turn_failure_settlements (
    execution TEXT NOT NULL,
    turn_id TEXT NOT NULL,
    failed_sequence INTEGER NOT NULL,
    outcome TEXT CHECK(outcome IS NULL OR outcome IN ('kept','session_ending')),
    sequence INTEGER,
    CHECK((outcome IS NULL) = (sequence IS NULL)),
    PRIMARY KEY (execution, turn_id)
);
