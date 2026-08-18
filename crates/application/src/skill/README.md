# Skill Application Module

This module implements transport-independent use cases for reusable skill records and their
on-disk packages.

## Responsibilities and boundaries

- Creation assigns a `SkillId`, applies backend timestamps, validates the domain entity, and
  atomically persists the database row together with a minimal `SKILL.md` under
  `<skills_root>/<name>/`.
- Get and list operations expose every visible catalog row and report `availability` from
  the on-disk package. A missing directory, missing root `SKILL.md`, or `SKILL.md` that
  cannot be loaded as YAML front matter marks the skill unavailable instead of deleting
  it or failing the request.
- Creating or importing the same name as an unavailable skill restores that row's package
  while keeping its identifier. Incomplete leftover directories may be cleared first. A
  complete untracked package is left in place at startup and when renaming onto that name;
  creating or importing an unclaimed name replaces any leftover at that path through a
  journaled swap so a failed persist restores the original package. Delete still succeeds
  when the formal directory is already gone.
- Update preserves identity and creation time, copies the existing package into a transaction
  staging directory, rewrites only the manifest (preserving unknown front matter values and the
  Markdown body), renames the formal directory when the name changes, and keeps the database and
  filesystem in sync atomically. Package files the user did not modify are preserved.
- Delete soft-deletes the record and moves the formal directory into a transaction backup
  atomically.
- `SkillStorage` isolates every filesystem mutation behind a statically dispatched port. The
  default `FilesystemSkillStorage` keeps staging, compensation backups, and journal markers under
  the reserved `<skills_root>/<.ora-staging|.ora-backup|.ora-journal>` directories so renames stay
  on one filesystem and interrupted transactions can be recovered at startup.
- `SkillRepository` supplies case-insensitive name lookups used for global uniqueness and import
  conflict detection.
- A destination claimed by another in-flight package transaction is a `SkillFolderConflict`,
  whether the journal is observed before promotion or the rename loses its final race. A static
  untracked package remains recoverable through a journaled swap, while a missing formal directory
  that a mutation still expected is `SkillStorageInconsistent`.
- Domain validation (`ora-domain::Skill`) enforces the ASCII slug name rules and the 4096-byte
  description limit shared by create, update, and import.

## Atomicity and recovery model

Atomicity across the two stores is provided by the journal and startup recovery, not by a database
transaction that spans both. Every mutation stages its package, writes a `Prepared` journal marker,
promotes the formal directory, marks the journal `Swapped`, writes the database row, and only then
clears the journal and its compensation artifacts. A failed database write compensates by rolling
the directory back from the backup.

Consequently no SQLite transaction is ever held open across a filesystem promote: the row write is
a single statement that runs after the rename returns. This bounds write-lock hold time to that one
statement, so a slow or network-backed skills root cannot starve unrelated catalog writers into the
pool's busy timeout. The cost is that a crash between the promote and the row write leaves a
recoverable inconsistency on disk rather than an impossible one, which is why every mutation must
write its journal marker before touching the formal tree and why startup blocks on
reconciliation before serving requests.

## Invariants

- Committed skill directory names and reserved transaction directory names occupy disjoint
  namespaces. Domain validation (`ora_domain::validate_skill_name`) refuses every dot-prefixed
  name, and this module's reserved directories (`STAGING_DIR_NAME`, `BACKUP_DIR_NAME`, and
  `JOURNAL_DIR_NAME`, re-exported from `ora-domain`) are all dot-prefixed. Otherwise a skill would
  be promoted onto a transaction root and startup reconciliation would delete its package while
  sweeping leftovers.
- A new reserved directory therefore only needs a leading dot to become unclaimable by a skill
  name — no separate allow/deny list to keep in sync.

See the [ora-application overview](../../README.md) and the
[skill_import module](../skill_import/README.md).
