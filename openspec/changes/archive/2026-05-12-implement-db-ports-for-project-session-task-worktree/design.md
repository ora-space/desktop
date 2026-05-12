## Context

`ora-application` already owns CRUD handlers and repository traits for `project`, `task`, `session`, and `worktree`, while `ora-db` currently owns SQLite bootstrap, migration reconciliation, and the base schema. The current schema already has tables for all four entity families, so the missing piece is the adapter layer that maps between `ora-application` ports, SQLite rows, and the existing `ora-domain` models.

The main constraint is preserving the existing architecture boundary: handlers remain transport-agnostic and persistence-agnostic, and `ora-db` becomes a concrete adapter that implements the application-owned ports. The schema is already fixed by migration `0001`, so the implementation must fit the current column layout, especially the soft-delete flag and integer-backed enum columns. The user also set explicit runtime requirements for SQLite connections: pooled access through `r2d2`, `journal_mode = WAL`, `busy_timeout = 5000`, and `synchronous = NORMAL`.

## Goals / Non-Goals

**Goals:**

- Add SQLite-backed implementations of `ProjectRepository`, `TaskRepository`, `SessionRepository`, and `WorktreeRepository` in `ora-db`.
- Reuse the existing `Database` bootstrap and connection-opening behavior while exposing repository adapters through an `r2d2` pool configured for the required SQLite pragmas.
- Persist and load the current `ora-domain` entities, including audit fields and enum conversions, without changing the domain or handler contracts.
- Cover the new repository adapters with deterministic `ora-db` tests that exercise CRUD-style behavior against SQLite.
- Update docs that currently describe `ora-db` persistence adapters as future work.

**Non-Goals:**

- Changing the database schema, adding new migrations, or enforcing new foreign-key constraints in this change.
- Moving use-case orchestration, validation, or logging ownership out of `ora-application`.
- Implementing repositories for entities outside the requested four, such as `artifact` or virtual folder models.
- Introducing a generic repository framework shared across all entity families.
- Changing the requested SQLite runtime settings away from WAL mode, `busy_timeout = 5000`, or `synchronous = NORMAL`.

## Decisions

### Add one repository module per entity plus shared SQLite helpers

`ora-db` will gain focused modules for `project`, `task`, `session`, and `worktree` repositories, plus a small shared internal helper area for repetitive SQLite concerns such as boolean encoding, audit field extraction, and row-to-domain mapping support.

Why:
- It mirrors the entity split already established in `ora-application` and keeps each module comfortably below the repo's size targets.
- Per-entity modules make it easier to review the SQL and mapping logic against the corresponding domain model and repository trait.

Alternative considered:
- Put all repository implementations into one large `repositories.rs` module.
  Rejected because the four entities have similar but still distinct column mappings, and one file would become hard to navigate quickly.

### Use an `r2d2` SQLite pool instead of a shared single connection

`ora-db` will expose repository adapters backed by an `r2d2` connection pool for SQLite rather than a single shared `Connection`. Each checked-out connection will be initialized with the required SQLite settings so repository operations run with WAL journaling, a `busy_timeout` of `5000`, and `synchronous = NORMAL`.

Why:
- The requested runtime profile explicitly calls for pooled access, and `r2d2` gives the application layer a simple synchronous integration surface that still fits the existing repository traits.
- WAL mode and a non-zero busy timeout are more useful when multiple operations can arrive concurrently, which aligns better with pooled connections than a single locked connection.
- `synchronous = NORMAL` provides the requested durability and performance trade-off while staying explicit in the adapter bootstrap path.

Alternative considered:
- Share one bootstrapped `Connection` behind `Arc<Mutex<Connection>>`.
  Rejected because it would serialize all repository work through one lock, would not honor the requested `r2d2` pooling decision, and would make the WAL-oriented concurrency settings much less meaningful.

### Configure SQLite connections centrally at pool creation time

`ora-db` will centralize SQLite connection initialization in the pool construction path. Every connection created for the pool must set `journal_mode = WAL`, `busy_timeout = 5000`, and `synchronous = NORMAL` before repositories start using it.

Why:
- These settings are part of the persistence contract for this slice, not incidental call-site details.
- Applying them centrally avoids subtle drift where some repositories or tests accidentally run with different SQLite behavior.

Alternative considered:
- Set PRAGMAs lazily inside repository methods.
  Rejected because it repeats setup work, makes behavior order-dependent, and increases the chance of misconfigured pooled connections.

### Keep repository adapters implementing application-owned traits directly

Each db repository type will implement the matching `ora-application` trait directly rather than introducing db-local traits or a second abstraction layer. Public exports from `ora-db` should make these adapter types straightforward to construct from the bootstrapped database.

Why:
- The application layer already defined the seam, and the proposal is specifically about filling that seam with SQLite-backed implementations.
- A direct implementation keeps testing and runtime wiring simple and avoids compatibility glue that would only exist temporarily.

Alternative considered:
- Add db-local traits and separate adapter structs that forward into them.
  Rejected because it adds indirection without improving testability or ownership.

### Preserve soft delete as an internal persistence detail

Visible reads in `ora-db` will consistently filter on `is_deleted = 0`, while `soft_delete_*` methods update `is_deleted` and `updated_at` instead of removing rows. The application layer continues to expose CRUD-shaped delete flows.

Why:
- This matches the current handler contracts and the schema already shipped in migration `0001`.
- Keeping the filtering centralized in repository queries prevents deleted rows from leaking back into normal application reads.

Alternative considered:
- Physically delete rows for the new repositories.
  Rejected because it would diverge from the schema intent and from the existing handler semantics.

### Treat row mapping as domain reconstruction, not DTO translation

The repository layer will reconstruct full `ora-domain` entities from SQLite rows, using the domain's existing `from_database_value` helpers for enum fields and preserving audit metadata exactly as stored.

Why:
- The application ports already speak in domain entities, so `ora-db` should stay aligned with that contract instead of inventing intermediate storage DTOs.
- Using the domain conversion helpers keeps invalid persisted enum values explicit and testable.

Alternative considered:
- Introduce db-only row structs and map through intermediate representations everywhere.
  Rejected because the extra indirection would add code without materially reducing complexity for this schema size.

## Risks / Trade-offs

- [WAL mode cannot be enabled for pure in-memory SQLite databases in the same way as file-backed databases] -> Mitigation: run WAL-specific assertions against file-backed test databases and keep in-memory tests for repository behavior that does not depend on journal persistence.
- [Pooled SQLite connections add another dependency and initialization path] -> Mitigation: keep pool construction in one small module and cover the configured PRAGMA behavior with integration-style tests.
- [Repository modules will contain similar SQL shapes across four entities] -> Mitigation: keep repetition intentional where it preserves clarity, and extract only the small helpers that genuinely reduce duplication.
- [Invalid persisted enum values will surface only when reading corrupt rows] -> Mitigation: map those failures into application-owned repository errors and cover representative bad-row cases in tests where practical.
- [No relationship validation means repositories can persist inconsistent foreign-key-like identifiers] -> Mitigation: document that this change preserves the current minimal scope and leave relationship-aware workflows for a later change.
