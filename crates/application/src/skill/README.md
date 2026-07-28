# Skill Application Module

This module implements transport-independent CRUD use cases for reusable skill records.

## Responsibilities and boundaries

- Creation assigns a `SkillId`, applies backend timestamps, validates the domain entity, and persists it.
- Get and list operations expose only visible records.
- Update preserves identity, creation time, and deletion state while replacing mutable skill data.
- Delete is a soft delete and distinguishes a missing visible skill from repository failure.
- Domain validation and repository errors are translated into stable `ApplicationError` variants.

`SkillRepository`, `SkillIdGenerator`, and `Clock` isolate storage, identity, and time from the handlers. The module maps domain entities to contract DTOs but does not parse skill files, install skills, execute skill instructions, or choose transport semantics.

See the [ora-application overview](../../README.md).
