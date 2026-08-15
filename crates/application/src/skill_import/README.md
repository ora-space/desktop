# Skill Import Application Module

This module owns the transport-independent two-phase import sessions that batch-install skills
from one folder tree or one supported archive (`.zip`, `.skill`, `.tar.gz`, `.tgz`).

## Responsibilities and boundaries

- Preparation (`prepare`) materializes one logical source into an OS-temp snapshot, validates all
  archive and path-safety constraints, discovers non-overlapping `SKILL.md` boundaries, parses
  each manifest, and queries the repository for existing names. It never mutates formal storage.
- Preview exposes every candidate with its safe source-relative path, file count, size, and
  `ready`/`conflict`/`invalid` status. `conflict` candidates carry the existing skill's identity
  and description for the user's `skip`/`overwrite` decision.
- Commit (`commit`) validates that every conflict candidate has a decision, freezes the decisions,
  and starts a detached background task that processes candidates in stable source-path order.
  Each skill is staged under the formal skills root and promoted atomically with its database
  write; one failed candidate never rolls back its siblings.
- Sessions live only in memory and expire by idle time or absolute lifetime. Completed or
  cancelled sessions retain lightweight result metadata for the result-retention window so
  idempotent retries and progress recovery keep working.
- `SkillImportProgressPublisher` is the reserved port for message-bus progress events; the default
  no-op publisher discards them until a bus adapter is wired. Large result objects are never sent
  through this channel.

## Non-responsibilities

This module does not read archives itself (`ora-skill-package` owns snapshot materialization,
path security, limits, scanning, and manifest parsing), does not implement the concrete
filesystem port (see the `skill` module), and does not decide HTTP or IPC semantics.

## Key invariants

- A session accepts exactly one logical source; mixing folders and archives, or multiple
  archives, is rejected.
- Within one source, two valid candidates declaring the same case-insensitive name fail the whole
  preparation.
- Once a commit is accepted it is uncancellable; retrying with the same decisions replays the
  stored results, and retrying with different decisions returns `already_committed`.
- Each candidate promotes its package directory before writing its database row, so no SQLite
  transaction stays open across the rename and a slow skills root cannot block unrelated writers.
  See the [skill module's atomicity and recovery model](../skill/README.md) for the journal and
  startup reconciliation that make this ordering crash-safe.

See the [ora-application overview](../../README.md).
