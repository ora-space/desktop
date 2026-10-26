CREATE TABLE execution_non_acceptance (
    execution TEXT PRIMARY KEY,
    operation TEXT NOT NULL UNIQUE,
    proof TEXT NOT NULL CHECK(json_valid(proof))
);
CREATE TRIGGER refuse_fenced_execution BEFORE INSERT ON execution_identities
WHEN EXISTS(SELECT 1 FROM execution_non_acceptance
    WHERE execution=NEW.execution OR operation=NEW.operation)
BEGIN SELECT RAISE(ABORT,'execution intake is permanently fenced'); END;
