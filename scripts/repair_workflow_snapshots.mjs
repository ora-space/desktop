import { randomUUID } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import { backup, DatabaseSync } from "node:sqlite";
import { fileURLToPath } from "node:url";
import { layoutWorkflowGraph } from "./workflow-qualification-suite/graph-layout.mjs";

export { layoutWorkflowGraph as repairGraph } from "./workflow-qualification-suite/graph-layout.mjs";

const ELIGIBLE_DRAFTS = `
  SELECT snapshot.id, snapshot.graph FROM workflow_snapshots snapshot
  JOIN workflows workflow ON workflow.id = snapshot.workflow_id
  WHERE snapshot.is_deleted = 0 AND workflow.is_deleted = 0
    AND snapshot.version = 'draft' AND snapshot.updated_at IS NOT NULL
    AND NOT EXISTS (SELECT 1 FROM workflows owner WHERE owner.published_snapshot_id = snapshot.id)
    AND NOT EXISTS (SELECT 1 FROM workflow_runs run WHERE run.snapshot_id = snapshot.id)
  ORDER BY snapshot.id
`;

/** Older or unrelated databases must fail closed instead of guessing snapshot eligibility. */
function validateDatabaseSchema(db) {
  const required = {
    workflow_snapshots: [
      "id",
      "workflow_id",
      "version",
      "graph",
      "updated_at",
      "is_deleted",
    ],
    workflows: ["id", "published_snapshot_id", "is_deleted"],
    workflow_runs: ["snapshot_id"],
  };
  for (const [table, columns] of Object.entries(required)) {
    const present = new Set(
      db
        .prepare(`PRAGMA table_info(${table})`)
        .all()
        .map((column) => column.name),
    );
    const missing = columns.filter((column) => !present.has(column));
    if (missing.length > 0)
      throw new Error(
        `Unsupported database schema: ${table} missing ${missing.join(", ")}`,
      );
  }
}

/**
 * Previews active unpinned draft repairs by default. Applying holds a write lock while an
 * online SQLite backup captures all committed WAL content, then commits every change together.
 */
export async function repairWorkflowSnapshots({
  databasePath,
  apply = false,
  backupPath,
}) {
  if (typeof databasePath !== "string" || databasePath.trim() === "") {
    throw new Error("An explicit --database path is required");
  }
  const database = path.resolve(databasePath);
  if (!fs.statSync(database).isFile())
    throw new Error("The --database path must name an existing SQLite file");
  const db = new DatabaseSync(database, { readOnly: !apply });
  let transaction = false;
  let savedBackup = null;
  try {
    validateDatabaseSchema(db);
    if (apply) {
      // Prevent another process from changing the source between backup and update selection.
      db.exec("BEGIN IMMEDIATE");
      transaction = true;
    }
    const rows = db.prepare(ELIGIBLE_DRAFTS).all();
    const changes = [];
    const rejected = [];
    for (const row of rows) {
      try {
        const graph = JSON.stringify(
          layoutWorkflowGraph(JSON.parse(row.graph)),
        );
        if (graph !== row.graph)
          changes.push({ id: row.id, previous: row.graph, graph });
      } catch (error) {
        rejected.push({ id: row.id, error: error.message });
      }
    }
    if (apply && rejected.length > 0) {
      throw new Error(`No repairs applied: ${JSON.stringify(rejected)}`);
    }
    if (apply && changes.length > 0) {
      const destination =
        backupPath === undefined
          ? `${database}.workflow-backup-${Date.now()}-${randomUUID()}.sqlite3`
          : path.resolve(backupPath);
      // Reserve a new file exclusively; the backup must never replace an existing user file.
      const reserved = fs.openSync(destination, "wx");
      fs.closeSync(reserved);
      let backupComplete = false;
      try {
        const source = new DatabaseSync(database, { readOnly: true });
        try {
          await backup(source, destination);
          backupComplete = true;
          savedBackup = destination;
        } finally {
          source.close();
        }
      } finally {
        if (!backupComplete) fs.unlinkSync(destination);
      }
      const update = db.prepare(
        "UPDATE workflow_snapshots SET graph = ? WHERE id = ? AND graph = ?",
      );
      for (const change of changes) {
        if (
          update.run(change.graph, change.id, change.previous).changes !== 1
        ) {
          throw new Error(`Draft ${change.id} changed during repair`);
        }
      }
    }
    if (transaction) {
      db.exec("COMMIT");
      transaction = false;
    }
    return {
      database,
      mode: apply ? "apply" : "dry-run",
      selected: rows.length,
      changed: changes.length,
      backup: savedBackup,
      rejected,
    };
  } catch (error) {
    if (transaction) db.exec("ROLLBACK");
    if (savedBackup)
      throw new Error(
        `${error.message}; original database backup: ${savedBackup}`,
        { cause: error },
      );
    throw error;
  } finally {
    db.close();
  }
}

/** Parses an explicit opt-in CLI; importing the module has no database side effects. */
async function main(args) {
  if (args.length === 1 && args[0] === "--help") {
    console.log(
      "Usage: node scripts/repair_workflow_snapshots.mjs --database <existing.sqlite3> [--apply] [--backup <new.sqlite3>]",
    );
    return;
  }
  const options = {};
  for (let index = 0; index < args.length; index++) {
    const argument = args[index];
    if (argument === "--apply" && options.apply === undefined) {
      options.apply = true;
    } else if (
      (argument === "--database" || argument === "--backup") &&
      args[index + 1] &&
      !args[index + 1].startsWith("--")
    ) {
      const key = argument === "--database" ? "databasePath" : "backupPath";
      if (options[key] !== undefined) throw new Error(`Duplicate ${argument}`);
      options[key] = args[++index];
    } else {
      throw new Error(`Unknown or incomplete argument: ${argument}`);
    }
  }
  if (options.backupPath !== undefined && !options.apply)
    throw new Error("--backup requires --apply");
  console.log(JSON.stringify(await repairWorkflowSnapshots(options), null, 2));
}

if (
  process.argv[1] &&
  fileURLToPath(import.meta.url) === path.resolve(process.argv[1])
) {
  main(process.argv.slice(2)).catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
