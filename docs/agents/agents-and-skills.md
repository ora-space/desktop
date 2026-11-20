# Agent Definitions and Skills

English | [中文](agents-and-skills.zh.md)

Besides the agent runtimes that plugins supply, Desktop keeps two user catalogs: **agent
definitions** (configurable roles a workflow node or chat can adopt) and **Skills** (reusable
instruction packages materialized into every Workspace). Both are namespaced by `(namespace, name)`
with case-insensitive visible names; local entries use the `local` namespace, and soft-deleted rows
do not reserve a name.

## Agent definitions

An `AgentDefinition` is a namespaced name, a description and Markdown content, stored in the
`agents` table. The `agent` operations are plain CRUD (`createAgent`, `getAgent`, `listAgents`,
`updateAgent`, `deleteAgent`); delete is a soft delete. Definitions are configuration records only:
they do not discover models, spawn CLIs or own session lifecycle. The Settings UI presents them as
Roles. Workflow Agent nodes reference them, and run creation freezes the resolved role into the run,
see [Workflow runs](../workflows/workflow-runs.md).

`prepareAgentImport` and `commitAgentImport` import a Markdown agent file (at most 1 MiB, YAML front
matter with `name` and `description`) into the `local` namespace. On a name conflict the commit
needs an explicit `skip` or `overwrite` decision plus the conflicting row's id and `updatedAt` from
preparation; if that row changed in between, the result is `stale_conflict` and nothing is written.

## Skills

A Skill is a catalog row plus an on-disk package `<data root>/atoms/skills/<name>/` whose root holds
`SKILL.md` with YAML front matter. `Skill.source` is `local` or `plugin { pluginId }`: plugins can
ship Skills, which are imported into the catalog with their origin recorded. Names are single ASCII
slug segments that never start with `.`, and descriptions are at most 4096 bytes.

Installed Skill plugins can declare public and secret settings for scripts. See
[Skill plugin configuration](skill-configuration.md) for declarations, startup environment snapshots,
restart behavior and security boundaries.

- `availability` is computed from the package: a missing directory or an unloadable `SKILL.md`
  marks the Skill unavailable instead of deleting it or failing the request. Creating or importing
  the same name restores that row's package and keeps its id.
- Create writes the row and a minimal `SKILL.md` together. Update stages a copy, rewrites only the
  manifest (keeping unknown front matter and the body), renames the directory when the name
  changes and keeps unmodified package files. Delete soft-deletes the row and moves the directory
  into a transaction backup.

### Package atomicity

Atomicity across SQLite and the filesystem comes from a journal and startup recovery, not from a
transaction spanning both. Every mutation stages its package under the reserved
`<skills root>/.ora-staging`, writes a `Prepared` marker under `.ora-journal`, promotes the formal
directory, marks the journal `Swapped`, writes the row and only then clears the journal and backups
(`.ora-backup`). A failed row write rolls the directory back. No SQLite transaction is held open
across a rename, so a slow Skills root cannot starve other writers.

Package files, journal markers and directory promotes are flushed before the row is committed, and
SQLite runs with `synchronous=FULL`, so the only crash window left is "directory exists, row not
committed", which startup reconciliation repairs by removing unowned packages. Mutation-time
directory fsync is a hard error on Unix and best effort on Windows. Startup blocks on this reconciliation before serving requests.

Details are in [`crates/application/src/skill/README.md`](../../crates/application/src/skill/README.md).

### Importing Skills

Skill import is a two-phase session over one folder tree or one archive (`.zip`, `.skill`,
`.tar.gz`, `.tgz`):

1. `prepareSkillImport` takes the local path chosen in the system picker (Desktop reads the files,
   so up to 200 MiB never crosses IPC), materializes a validated snapshot in OS temp storage,
   discovers non-overlapping `SKILL.md` boundaries and parses each manifest. It never touches formal
   storage.
2. `getSkillImport` previews every candidate as `ready`, `conflict` (an existing Skill with a usable
   package, offering `skip` or `overwrite`) or `invalid`.
3. `commitSkillImport` requires a decision for every conflict, freezes the decisions and imports in
   the background in stable path order. Each Skill is promoted atomically with its row, and one
   failure never rolls back its siblings. An accepted commit cannot be cancelled; retrying with the
   same decisions replays the stored results.
4. `cancelSkillImport` ends a session that has not committed.

Sessions live in memory and expire; completed sessions keep lightweight results for a retention
window. Archive safety (zip-slip, encrypted and special entries, portable case conflicts, entry and
byte budgets) belongs to `ora-utils::archive` and `ora-utils::path`; Skill limits and manifest
parsing belong to `ora-skill-package`. Plugin surfaces can trigger the same import from a download,
see [Surfaces](../plugins/surfaces.md#downloads).

## How Skills reach agents

Installed Skills are materialized into every eligible Workspace by the Effect system, see
[Effects](effects.md). A workflow Agent node's Skill bindings mean "required invocation" in that
node's prompt; they are not a node-level allowlist and do not hide other materialized Skills.
