-- Widen the common identity check without renaming a table referenced by existing records.
CREATE TEMP TABLE saved_revision_identities AS SELECT * FROM execution_identities;
DROP TABLE execution_identities;
CREATE TABLE execution_identities (
    operation TEXT PRIMARY KEY,
    execution TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL CHECK(kind IN ('worktree','clone','plugin','agent_session','deliver_revision'))
);
INSERT INTO execution_identities SELECT * FROM saved_revision_identities;
DROP TABLE saved_revision_identities;
-- A delivery is admitted already started (its permit is checked at admission), so it has no
-- accepted state. The frozen plan exists only while running and is dropped with the terminal result.
CREATE TABLE revision_deliveries (
    execution TEXT PRIMARY KEY REFERENCES execution_identities(execution),
    input TEXT NOT NULL CHECK(json_valid(input)),
    state TEXT NOT NULL CHECK(state IN ('running','completed')),
    plan TEXT CHECK(plan IS NULL OR json_valid(plan)),
    result TEXT CHECK(result IS NULL OR json_valid(result)),
    CHECK((state='completed') = (result IS NOT NULL)),
    CHECK(state='running' OR plan IS NULL)
);
CREATE TABLE revision_outbox (
    execution TEXT PRIMARY KEY REFERENCES revision_deliveries(execution),
    event TEXT NOT NULL CHECK(json_valid(event))
);
CREATE TRIGGER bind_new_revision AFTER INSERT ON revision_deliveries BEGIN
    INSERT INTO execution_controllers SELECT NEW.execution,controller FROM controller_binding;
END;
