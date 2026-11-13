# Testing

English | [中文](testing.zh.md)

The coding and assertion rules for tests are in the repository's `AGENTS.md`; this document lists the
gates and what each covers.

## Task groups

| Task | Covers |
|---|---|
| `task test:frontend` | `check:contracts`, then lint and tests of every frontend package, including the Desktop TypeScript transport |
| `task test:crates` | The reusable Rust crates |
| `task test:tauri` | The Tauri Desktop package |
| `task test:e2e` | Desktop end-to-end tests (`e2e/`, with a fake ACP agent) |
| `task test` | All of the above plus every lint; long-running |
| `task lint`, `lint:frontend`, `lint:crates`, `lint:tauri`, `lint:e2e` | Formatting, Clippy and linters |
| `task check:contracts` | Generated contracts and plugin protocol bindings match their sources |
| `task check:features` | Feature interface visibility, see [Frontend architecture](../core/frontend-architecture.md#public-feature-interfaces) |
| `task check:rust-size` | Production Rust module size with ratcheting exceptions (`xtask/rust-size-baseline.json`) |
| `task proto:check` | The Cloud contract submodule is pinned and the generated client matches it (part of `lint:crates`) |
| `deno task test:tooling` | Tooling scripts under `scripts/` |

The Tauri crate shares the root `Cargo.lock`, dependency graph and target directory with the other Rust
crates. CI runs the groups independently, and the crates job runs on Linux, macOS and Windows. Use the
smallest relevant task while iterating; Desktop changes need `task test:tauri`, not only workspace tests,
and cross-cutting changes finish with `task test`.

## Frontend

Frontend tests run under `scripts/run-with-clean-stderr.ts`: any warning on stderr, notably an act()
warning, fails the run even when Vitest is green. Tests use the production generated client with typed
operation handlers (see [Frontend architecture](../core/frontend-architecture.md#typed-test-transport)),
import `appI18n` themselves when they render translations, and await every operation that can update
React at its real asynchronous boundary. The contracts, chat and editor packages run `tsc --noEmit` and
then `deno test --no-check`.

## Rust

Tests prefer production interfaces with real SQLite and real Git where those are the behavior under
test, inject clocks and dependencies instead of mutating the process environment, and compare whole
objects with `pretty_assertions::assert_eq`. Code that emits `tracing` events runs under the
thread-scoped TRACE helpers, see [Logging](../core/logging.md#testing).

Some suites need extra setup:

- Node standalone tests run the real Node, Controller and reaper executables; build them first with
  `cargo build -p ora-node -p ora-controller -p ora-reaper`.
- Clone, delivery, restore and `ora-git` tests run a local Git Server double with client-certificate
  TLS backed by `git http-backend` (the `git-server` feature of `ora-test-support`), built only on
  Linux.
