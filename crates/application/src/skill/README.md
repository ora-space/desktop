# Skill Application Module

This module implements transport-independent use cases for reusable skill records and their
on-disk packages.

## Responsibilities and boundaries

- Creation assigns a `SkillId`, applies backend timestamps, validates the domain entity, and
  atomically persists the database row together with a minimal `SKILL.md` under
  `<skills_root>/<name>/`.
- Get and list operations expose only visible records.
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

See the [ora-application overview](../../README.md) and the
[skill_import module](../skill_import/README.md).
