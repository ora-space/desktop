import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { DatabaseSync } from "node:sqlite";
import test from "node:test";
import { fileURLToPath } from "node:url";

const script = fileURLToPath(
  new URL("./repair_workflow_snapshots.mjs", import.meta.url),
);

/** Restricts every subprocess and cleanup to a disposable database directory. */
function fixture(t) {
  const root = fs.mkdtempSync(
    path.join(os.tmpdir(), "ora-workflow-repair-test-"),
  );
  const fakeAppData = path.join(root, "unused-appdata");
  fs.mkdirSync(path.join(fakeAppData, "space.ora.desktop"), {
    recursive: true,
  });
  const database = path.join(root, "fixture.sqlite3");
  const connections = [];
  t.after(() => {
    for (const db of connections) if (db.isOpen) db.close();
    assert.equal(path.dirname(path.resolve(root)), path.resolve(os.tmpdir()));
    fs.rmSync(root, { recursive: true, force: true });
  });
  return { root, fakeAppData, database, connections };
}

/** Gives the maintenance CLI a fake profile so old implementations cannot touch real data. */
function runCli(temp, args) {
  const runtimeArgs = globalThis.Deno
    ? [
        "run",
        "--no-config",
        "--no-lock",
        `--allow-read=${temp.root}`,
        `--allow-write=${temp.root}`,
        script,
        ...args,
      ]
    : [script, ...args];
  return spawnSync(process.execPath, runtimeArgs, {
    encoding: "utf8",
    env: { ...process.env, APPDATA: temp.fakeAppData },
    timeout: 15000,
  });
}

/** Mirrors only the production columns the repair tool must use to protect snapshots. */
function seedDatabase(database) {
  const db = new DatabaseSync(database);
  db.exec(`
    PRAGMA journal_mode = WAL;
    PRAGMA wal_autocheckpoint = 0;
    CREATE TABLE workflows (id TEXT PRIMARY KEY, published_snapshot_id TEXT, is_deleted INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE workflow_snapshots (id TEXT PRIMARY KEY, workflow_id TEXT NOT NULL, version TEXT NOT NULL, graph TEXT NOT NULL, updated_at INTEGER, is_deleted INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE workflow_runs (id TEXT PRIMARY KEY, snapshot_id TEXT NOT NULL, is_deleted INTEGER NOT NULL DEFAULT 0);
    CREATE TABLE unrelated_records (value TEXT);
    INSERT INTO workflows (id, published_snapshot_id) VALUES ('active', 'published');
    INSERT INTO workflows (id, is_deleted) VALUES ('deleted-workflow', 1);
    INSERT INTO unrelated_records VALUES ('committed in the WAL');
  `);
  const graph = JSON.stringify({
    nodes: [
      { id: "start", data: { kind: "start", input: "preserved" } },
      {
        id: "agent",
        data: {
          kind: "agent",
          agentConfig: { prompt: "author intent", custom: { retained: true } },
        },
      },
    ],
    edges: [{ source: "start", target: "agent" }],
  });
  const insert = db.prepare(
    "INSERT INTO workflow_snapshots VALUES (?, ?, ?, ?, ?, ?)",
  );
  for (const row of [
    ["draft", "active", "draft", graph, 1, 0],
    ["published", "active", "v1", graph, null, 0],
    ["old-published", "active", "v0", graph, 1, 0],
    ["deleted-draft", "active", "draft", graph, 1, 1],
    ["immutable-draft", "active", "draft", graph, null, 0],
    ["pinned-draft", "active", "draft", graph, 1, 0],
    ["orphan-draft", "deleted-workflow", "draft", graph, 1, 0],
  ])
    insert.run(...row);
  db.prepare("INSERT INTO workflow_runs VALUES (?, ?, ?)").run(
    "old-run",
    "pinned-draft",
    1,
  );
  return db;
}

/** Reads complete rows so tests notice accidental changes outside the graph column. */
function rows(db) {
  return db.prepare("SELECT * FROM workflow_snapshots ORDER BY id").all();
}

test("importing the repair module does not open a database or emit output", (t) => {
  const temp = fixture(t);
  const expression = `await import(${JSON.stringify(new URL("./repair_workflow_snapshots.mjs", import.meta.url).href)})`;
  const runtimeArgs = globalThis.Deno
    ? ["eval", "--no-config", "--no-lock", expression]
    : ["--input-type=module", "-e", expression];
  const result = spawnSync(process.execPath, runtimeArgs, {
    encoding: "utf8",
    env: { ...process.env, APPDATA: temp.fakeAppData },
    timeout: 15000,
  });
  assert.equal(result.status, 0, result.stderr);
  assert.equal(result.stdout, "");
  assert.deepEqual(
    fs.readdirSync(path.join(temp.fakeAppData, "space.ora.desktop")),
    [],
  );
});

test("the CLI requires an explicit existing database path", (t) => {
  const temp = fixture(t);
  const missing = runCli(temp, []);
  assert.notEqual(missing.status, 0);
  assert.match(missing.stderr, /--database/);
  const absent = runCli(temp, ["--database", temp.database]);
  assert.notEqual(absent.status, 0);
  assert.equal(fs.existsSync(temp.database), false);
  assert.deepEqual(
    fs.readdirSync(path.join(temp.fakeAppData, "space.ora.desktop")),
    [],
  );
});

test("default dry-run leaves all rows unchanged and previews only active unpinned drafts", (t) => {
  const temp = fixture(t);
  const db = seedDatabase(temp.database);
  temp.connections.push(db);
  const before = rows(db);
  const result = runCli(temp, ["--database", temp.database]);
  assert.equal(result.status, 0, result.stderr);
  const report = JSON.parse(result.stdout);
  assert.deepEqual(
    {
      mode: report.mode,
      selected: report.selected,
      changed: report.changed,
      backup: report.backup,
    },
    { mode: "dry-run", selected: 1, changed: 1, backup: null },
  );
  assert.deepEqual(rows(db), before);
});

test("apply backs up WAL contents and changes only draft layout without inventing configuration", (t) => {
  const temp = fixture(t);
  const db = seedDatabase(temp.database);
  temp.connections.push(db);
  const before = rows(db);
  const result = runCli(temp, ["--database", temp.database, "--apply"]);
  assert.equal(result.status, 0, result.stderr);
  const report = JSON.parse(result.stdout);
  assert.equal(report.mode, "apply");
  assert.equal(report.changed, 1);
  assert.ok(report.backup);
  const saved = new DatabaseSync(report.backup, { readOnly: true });
  try {
    assert.deepEqual(rows(saved), before);
    assert.equal(
      saved.prepare("SELECT value FROM unrelated_records").get().value,
      "committed in the WAL",
    );
    assert.equal(
      saved.prepare("PRAGMA integrity_check").get().integrity_check,
      "ok",
    );
  } finally {
    saved.close();
  }
  const after = rows(db);
  assert.deepEqual(
    after.filter((row) => row.id !== "draft"),
    before.filter((row) => row.id !== "draft"),
  );
  const draft = JSON.parse(after.find((row) => row.id === "draft").graph);
  assert.deepEqual(draft.nodes[1].data.agentConfig, {
    prompt: "author intent",
    custom: { retained: true },
  });
  assert.ok(draft.edges[0].id);
  assert.ok(
    draft.nodes.every(
      (node) =>
        Number.isFinite(node.position.x) && Number.isFinite(node.position.y),
    ),
  );
  const again = runCli(temp, ["--database", temp.database, "--apply"]);
  assert.equal(again.status, 0, again.stderr);
  assert.equal(JSON.parse(again.stdout).changed, 0);
  assert.equal(JSON.parse(again.stdout).backup, null);
});

test("a failed update rolls back every selected draft and closes its database", (t) => {
  const temp = fixture(t);
  const db = seedDatabase(temp.database);
  db.exec(
    "INSERT INTO workflow_snapshots SELECT 'fail-draft', workflow_id, version, graph, updated_at, is_deleted FROM workflow_snapshots WHERE id = 'draft'; CREATE TRIGGER reject_repair BEFORE UPDATE OF graph ON workflow_snapshots WHEN OLD.id = 'fail-draft' BEGIN SELECT RAISE(ABORT, 'fixture rejects second update'); END;",
  );
  const before = rows(db);
  db.close();
  const result = runCli(temp, ["--database", temp.database, "--apply"]);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /fixture rejects second update/);
  const reopened = new DatabaseSync(temp.database);
  try {
    assert.deepEqual(rows(reopened), before);
    reopened.exec("BEGIN IMMEDIATE; ROLLBACK;");
  } finally {
    reopened.close();
  }
  assert.equal(
    fs.readdirSync(temp.root).filter((name) => name.includes("backup")).length,
    1,
  );
});

test("ambiguous ownership is reported and apply makes no changes", (t) => {
  const temp = fixture(t);
  const db = seedDatabase(temp.database);
  const malformed = JSON.stringify({
    nodes: [{ id: "agent", parentId: "missing", data: { kind: "agent" } }],
    edges: [],
  });
  db.prepare("UPDATE workflow_snapshots SET graph = ? WHERE id = 'draft'").run(
    malformed,
  );
  const before = rows(db);
  db.close();
  const preview = runCli(temp, ["--database", temp.database]);
  assert.equal(preview.status, 0, preview.stderr);
  assert.match(
    JSON.parse(preview.stdout).rejected[0].error,
    /parent|owner|container/i,
  );
  const applied = runCli(temp, ["--database", temp.database, "--apply"]);
  assert.notEqual(applied.status, 0);
  const reopened = new DatabaseSync(temp.database, { readOnly: true });
  try {
    assert.deepEqual(rows(reopened), before);
  } finally {
    reopened.close();
  }
});

test("an existing backup destination is preserved and no draft update starts", (t) => {
  const temp = fixture(t);
  const db = seedDatabase(temp.database);
  const before = rows(db);
  db.close();
  const existing = path.join(temp.root, "existing-backup.sqlite3");
  fs.writeFileSync(existing, "Keep this user file");
  const result = runCli(temp, [
    "--database",
    temp.database,
    "--apply",
    "--backup",
    existing,
  ]);
  assert.notEqual(result.status, 0);
  assert.equal(fs.readFileSync(existing, "utf8"), "Keep this user file");
  const reopened = new DatabaseSync(temp.database);
  try {
    assert.deepEqual(rows(reopened), before);
    reopened.exec("BEGIN IMMEDIATE; ROLLBACK;");
  } finally {
    reopened.close();
  }
});

test("an unsupported database schema is rejected without creating tables", (t) => {
  const temp = fixture(t);
  const db = new DatabaseSync(temp.database);
  db.exec(
    "CREATE TABLE unrelated_records (value TEXT); INSERT INTO unrelated_records VALUES ('untouched');",
  );
  db.close();
  const result = runCli(temp, ["--database", temp.database, "--apply"]);
  assert.notEqual(result.status, 0);
  assert.match(result.stderr, /Unsupported database schema/);
  const reopened = new DatabaseSync(temp.database, { readOnly: true });
  try {
    assert.equal(
      reopened.prepare("SELECT value FROM unrelated_records").get().value,
      "untouched",
    );
    assert.deepEqual(
      reopened
        .prepare("SELECT name FROM sqlite_master WHERE type = 'table'")
        .all()
        .map((row) => row.name),
      ["unrelated_records"],
    );
  } finally {
    reopened.close();
  }
});
