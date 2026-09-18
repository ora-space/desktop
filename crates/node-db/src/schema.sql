CREATE TABLE executions (
    operation TEXT PRIMARY KEY,
    execution TEXT NOT NULL UNIQUE,
    input TEXT NOT NULL CHECK(json_valid(input)),
    target TEXT CHECK(target IS NULL OR json_valid(target)),
    state TEXT NOT NULL CHECK(state IN ('accepted','running','unknown','completed')),
    progress TEXT NOT NULL CHECK(json_valid(progress) AND json_extract(progress, '$.kind') = state),
    CHECK(operation = json_extract(input, '$.message.operation_id')),
    CHECK(execution = json_extract(input, '$.message.execution_id')),
    CHECK(state = 'completed' OR target IS NOT NULL)
);
CREATE TABLE resources (
    worktree TEXT PRIMARY KEY,
    workspace TEXT NOT NULL,
    repository TEXT NOT NULL,
    path TEXT NOT NULL,
    branch TEXT NOT NULL,
    active INTEGER NOT NULL CHECK(active IN (0,1)),
    data TEXT NOT NULL CHECK(json_valid(data)),
    owner_execution TEXT NOT NULL REFERENCES executions(execution)
);
CREATE UNIQUE INDEX resource_path ON resources(path) WHERE active = 1;
CREATE UNIQUE INDEX resource_branch ON resources(repository, branch) WHERE active = 1;
CREATE UNIQUE INDEX resource_workspace ON resources(workspace) WHERE active = 1;
CREATE TABLE outbox (
    execution TEXT PRIMARY KEY REFERENCES executions(execution),
    event TEXT NOT NULL CHECK(json_valid(event))
);
