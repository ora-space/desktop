# Skill Application Module

This module implements transport-independent CRUD and atomic folder-import use cases for reusable skills.

## Responsibilities and boundaries

- Creation assigns a `SkillId`, applies backend timestamps, validates the domain entity, and persists it.
- Get and list operations expose only visible records.
- Update preserves identity, creation time, and deletion state while replacing mutable skill data.
- Delete is a soft delete and distinguishes a missing visible skill from repository failure.
- Folder import validates bounded file sets and root `SKILL.md` front matter, stages files under an identifier-owned temporary directory, and commits the catalog row and package promotion as one unit of work.
- Startup reconciliation removes abandoned staging directories and committed packages with no visible catalog row.
- Domain validation becomes stable semantic `ApplicationError` variants. Repository, filesystem, and transaction failures preserve their concrete `Error::source()` chains instead of storing formatted messages.
- A compensating cleanup failure is emitted once as a bounded secondary error event and never replaces the primary import failure completed by the runtime request seam.

`SkillRepository`, `SkillPackageStore`, `SkillImportUnitOfWork`, `SkillIdGenerator`, and `Clock` isolate catalog storage, package storage, atomic commit, identity, and time from the handlers. `LocalSkillPackageStore` owns the `<data-root>/atoms/skills` layout. The module maps domain entities to contract DTOs but does not execute skill instructions or choose HTTP status codes.

See the [ora-application overview](../../README.md).
