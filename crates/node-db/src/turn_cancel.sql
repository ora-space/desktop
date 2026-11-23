CREATE TABLE session_turn_cancels (
    execution TEXT NOT NULL REFERENCES node_executions(execution),
    command_id TEXT NOT NULL CHECK(length(command_id)>0),
    turn_id TEXT NOT NULL CHECK(length(turn_id)>0),
    outcome TEXT NOT NULL CHECK(outcome IN ('stopped','not_started','already_ended','unknown','session_ending')),
    sequence INTEGER NOT NULL,
    PRIMARY KEY(execution,command_id)
);
