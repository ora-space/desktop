# ora-application

`ora-application` is Ora's transport- and storage-independent use-case layer. It coordinates domain models, contract DTOs, repositories, clocks, identifier generators, and Git worktree provisioning without depending on SQLite, HTTP, or Tauri.

## Module map

- [agent_definition](src/agent_definition/README.md) manages configurable agent definitions.
- [project](src/project/README.md) manages project creation, reads, and rename operations.
- [session](src/session/README.md) provides persisted session reads and soft deletion.
- [skill](src/skill/README.md) manages reusable skill records, their atomic on-disk packages, and journaled filesystem transactions.
- [skill_import](src/skill_import/README.md) owns the two-phase batch import sessions over folder and archive sources.
- [task](src/task/README.md) coordinates task persistence and optional Git worktree creation.
- [task_diff](src/task_diff/README.md) coordinates task-scoped diff review, Git writes, and anchored discussions.
- [worktree](src/worktree/README.md) defines persistence and identity ports for task-owned worktrees.
- [workflow](src/workflow/README.md) manages workflow definitions and versioned snapshots.
- [workflow_run](src/workflow_run/README.md) manages run CRUD; execution lives in [workflow_run/engine](src/workflow_run/engine/README.md).

## Boundaries

Handlers accept `ora-contracts` requests, operate on `ora-domain` models, and map results back to contract responses. Infrastructure is injected through statically dispatched repository, clock, identifier, worktree, and task-diff traits so use-case rules can be tested without a database or Git process.

`ApplicationError` is the stable application-facing failure vocabulary. Handlers translate domain and skill-package validation into semantic variants and retain infrastructure failures as `Error::source()` chains through `RepositoryError`, task-diff port errors, and `SkillPackageStoreError`. Handlers do not emit generic success or propagation-only failure events, choose public error codes, or select transport status. Web, Tauri, and stream seams own the single correlated request-completion event and derive its level from the public classification. Skill-import compensation is the deliberate exception: a cleanup failure is a separate lifecycle fact and is logged without replacing the primary error.

Aggregate deletion, SQLite composition, ACP process supervision, and transport-neutral public error normalization belong to `ora-backend` and `ora-db`. Contract serialization and endpoint metadata belong to `ora-contracts`.

See [Application and Contracts Boundary](../../docs/application-contracts.md) for the cross-crate ownership model.
