-- A provider may have created a session after admission, even when its history was lost.
CREATE TABLE session_history_admissions (
    execution TEXT PRIMARY KEY REFERENCES node_executions(execution)
);
-- Absence of an old admission record proves nothing. Retain uncertainty for every old execution.
INSERT INTO session_history_admissions(execution) SELECT execution FROM node_executions;
