CREATE TABLE managed_executions (
    execution TEXT PRIMARY KEY REFERENCES executions(execution)
);
CREATE TABLE process_attempts (
    run TEXT PRIMARY KEY,
    execution TEXT NOT NULL REFERENCES managed_executions(execution),
    data BLOB NOT NULL,
    cleaned INTEGER NOT NULL CHECK(cleaned IN (0,1))
);
