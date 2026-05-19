## Context

`ora-domain` and the initial migration already define `Task`, `Worktree`, and `Session`, but `ora-application` and `ora-contracts` currently stop at the first `project` vertical slice. The existing `project` implementation provides a clear pattern: transport-agnostic handlers, handler-owned ports, contract mapping, structured logs, and focused unit tests with in-memory fakes.

This change extends that pattern to the remaining three schema-backed entities that matter for near-term IDE workflows. The user specifically wants the simplest usable handler layer first, so this design keeps cross-model validation out of scope even when foreign-key-like identifiers appear in request payloads.

## Goals / Non-Goals

**Goals:**

- Add `task`, `worktree`, and `session` CRUD request and response contracts in `ora-contracts`.
- Add `Create*`, `Get*`, `List*`, `Update*`, and `Delete*` handlers for each entity in `ora-application`.
- Keep handler orchestration transport-agnostic and unit-testable through application-owned traits and in-memory fakes.
- Preserve the existing `project` slice conventions for mapping, logging, soft-delete behavior, and module exports.
- Make the first implementation incrementally shippable without changing the database schema.

**Non-Goals:**

- Enforcing relationship integrity such as confirming a referenced `project_id`, `task_id`, or `worktree_id` exists before create or update operations.
- Introducing richer query handlers, filtered listings, or workflow-specific commands beyond the basic five CRUD-style handlers per entity.
- Changing `ora-domain` entity shapes or adding new database tables and columns.
- Designing frontend screens or adapter-specific routes and commands in this change.

## Decisions

### Mirror the `project` vertical slice with one module per entity

`ora-application` will gain `task`, `worktree`, and `session` modules, each with `handlers.rs`, `mapper.rs`, `ports.rs`, and `tests.rs`, plus optional ID generator support where create flows need new identifiers. `ora-contracts` will gain one file per entity and re-export those contracts from `lib.rs`.

Why:
- This keeps each module below the repo's preferred size limits and makes the new code easy to compare against the already-tested `project` slice.
- A per-entity layout keeps traits and fake repositories local to the relevant use cases instead of growing one shared abstraction layer too early.

Alternative considered:
- Generalize all CRUD handlers behind one reusable generic helper framework.
  Rejected because it would add abstraction pressure before the repository traits and error shapes for the new entities are even established.

### Extend application errors and repository error mapping per entity

`ApplicationError` will expand beyond project-only variants so each handler family can return a stable not-found error and a repository-operation error that names the relevant entity. The application layer will also add repository error enums for `task`, `worktree`, and `session` alongside the existing project port errors.

Why:
- Entity-specific error variants preserve the adapter-facing clarity already present in the `project` slice.
- Keeping mapping in `ora-application` prevents database or transport crates from owning business-facing failure semantics.

Alternative considered:
- Collapse all missing-entity outcomes into a single generic not-found variant.
  Rejected because it would make logs and adapter translations less explicit and would diverge from the current project behavior.

### Keep public contracts flat and serialization-friendly

Each entity gets one shared public view model plus CRUD DTOs modeled after `project`. The first slice will expose persisted identifier and business fields only:
- `Task`: `id`, `project_id`, `title`, `status`, `worktree_id`
- `Worktree`: `id`, `task_id`, `branch_name`, `activity`
- `Session`: `id`, `task_id`, `agent_id`, `agent_session_id`, `status`

Audit fields remain internal to the domain and repository layers.

Why:
- This matches the existing contract philosophy and keeps future frontend type generation stable.
- A single shared view type per entity avoids premature summary/detail variants.

Alternative considered:
- Reuse domain entities directly as contract payloads.
  Rejected because it would leak internal audit fields and couple frontend types to domain evolution.

### Do not validate cross-model relationships in the first slice

Handlers will treat foreign-key-like identifiers as typed values to persist and retrieve, but they will not verify that related rows exist during create or update operations. Repositories remain responsible for persistence only, not orchestration across entity families.

Why:
- This matches the user's requested scope and keeps the first handler set small enough to land quickly.
- It avoids inventing coordination traits between modules before the product needs relationship-aware workflows.

Alternative considered:
- Add repository lookups across `project`, `task`, and `worktree` during create and update flows.
  Rejected because it would couple the new modules together, expand the test surface substantially, and exceed the goal of a minimal first slice.

### Reuse the project logging model with entity-specific context

Each new handler family will emit structured success and failure events from `ora-application`, using operation names like `create_task` or `delete_session` plus the relevant entity identifier when available.

Why:
- This preserves observability consistency across handler families.
- The existing project tests already establish a practical shape for validating structured events.

Alternative considered:
- Defer logging until adapters are added.
  Rejected because the project slice already treats logging as an application-layer responsibility, and consistency matters more than saving a small amount of code now.

## Risks / Trade-offs

- [Three parallel CRUD modules increase code repetition] -> Mitigation: keep the repetition intentional and pattern-based for now so the first implementation stays easy to review and refactor later.
- [Entity-specific errors enlarge `ApplicationError`] -> Mitigation: group mapping helpers by entity and keep adapters bound to one shared error enum instead of scattering conversions.
- [Skipping relationship validation can allow inconsistent identifiers] -> Mitigation: document the constraint in specs and tasks so a later change can add relationship-aware workflows without surprising callers.
- [Adding multiple public DTO families at once increases test volume] -> Mitigation: mirror the existing project test structure and favor whole-object assertions with in-memory fakes for fast feedback.
