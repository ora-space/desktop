# ora-utils

`ora-utils` is the lowest-level Rust crate in the workspace: generic, domain-free building blocks
that any other crate can consume without introducing dependency cycles.

## Responsibilities and boundaries

- `path`: platform-independent relative-path parsing (`PortableRelativePath`,
  `StrictRelativePath`), canonical root containment (`CanonicalPathRoot`), and lexical
  normalization helpers.
- `archive` (Cargo feature `archive`): safe materialization of untrusted `.zip` / `.tar.gz`
  archives and folder trees into a destination directory with zip-slip defenses, encrypted and
  special-entry rejection, portable case-conflict detection, and cumulative entry/byte budgets.

## Non-responsibilities

- No `ora-*` dependencies and no domain vocabulary (skills, plugins, tasks, workspaces). Callers
  wrap these primitives with their own semantics and error codes.
- No runtime dependencies (no async runtime, no watchers, no subprocesses).
- No workspace-level filesystem services; those live in `ora-fs`.

## Admission rule

Logic belongs here when it is independent of every Ora domain concept, transport, and runtime,
already has one consumer, and a second consumer could use it unchanged. Heavier optional
dependencies must be gated behind Cargo features so path-only consumers stay light.
