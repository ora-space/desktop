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

See the [ora-application overview](../../README.md) and the
[skill_import module](../skill_import/README.md).
