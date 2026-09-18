# Durable repository acquisition

English | [中文](repository-acquisition.zh.md)

Clone has separate business records in the existing explicitly injected Node database. It does not
reinterpret Worktree inputs or terminal results. The Linux runtime now connects storage, native
directory identity, managed Git acquisition and restart reconciliation; see [deployment](../repository-clone.md).

`accept_clone` atomically stores the original command, generated repository identity, root/path and
initial reservation. It never creates a directory. Replaying the same input returns the original target;
changed input or cross-capability operation/execution reuse fails. Destinations cannot overlap any active
Worktree target or any retained clone target. Clone failure never releases its reservation.

Progress retains Reserved, DirectoryCreated or Dispatched facts. Directory identity is frozen after
creation; Unknown retains its phase and cannot roll back to Reserved. SQLite treats native identity as
opaque evidence: a filesystem owner must capture and verify it, not infer ownership from this row.
`ProcessJournal::manage_clone` starts only after durable directory creation evidence. At most one mutation
Run can be recorded per clone execution. Retrying a failed clone requires new identities and a new target.

Terminal result and original event commit in one transaction. Result input/resource/path must match the
original record. A dispatched attempt requires exactly one observed exit code and no uncleaned Runs;
success requires code zero; verification may reject a code-zero tag checkout as a missing branch.
Missing exit/cleanup evidence remains recoverable,
not terminal. Native repository fact checks are an additional runtime obligation, not supplied by SQLite.
Pre-dispatch failures may complete with appropriate owned-residue evidence without inventing a Run.

Queries do not acknowledge events. `pending_events` combines both businesses; acknowledgement checks the
original operation/execution, Node and sequence 1. Only the delivery record is removed; retained results,
input and directory reservation survive. Completion and outbox write failures roll back together.

Schema v3 migrates recognized v1/v2 tables transactionally, preserves old Worktree records/events and
unfinished process attempts, and rejects unknown definitions. Existing old binaries refuse the new version.
`tests/repository.rs` and `tests/repository_migration.rs` cover these storage obligations using real SQLite;
they do not prove remote Git execution, directory ownership, Controller takeover or old-binary execution.
