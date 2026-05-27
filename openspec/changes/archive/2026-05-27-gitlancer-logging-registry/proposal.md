## Why

`gitlancer` is used as a library by higher-level crates, but it currently has no observability: callers cannot see which git commands are executed, how long they take, or whether they fail. Adding an opt-in logger registry lets integrators attach structured logging without forcing every downstream user of the crate to take an `ora-logging` dependency.

## What Changes

- **gitlancer** gains an optional logger registry: a trait `GitlancerLogger` and a `register_logger` function backed by a `OnceLock`.
- `CliGitRunner::run()` emits command and result events through the registered logger when one is present; the calls are no-ops otherwise.
- **ora-logging** gains a `OraGitlancerLogger` struct that implements `GitlancerLogger` by forwarding to `tracing` macros (`ora_info!`, `ora_error!`) and a `register_gitlancer_logger()` convenience function so callers can wire the two crates together in one line.

## Capabilities

### New Capabilities

- `gitlancer-logger-registry`: Optional structured-logging hook for gitlancer git command execution, including command args, duration, exit code, and error details.

### Modified Capabilities

<!-- No existing spec-level requirements change. -->

## Impact

- `crates/gitlancer`: adds a new `logging` module; `CliGitRunner` gains logger calls; no breaking API changes.
- `crates/logging`: adds `OraGitlancerLogger` and `register_gitlancer_logger`; depends on a new `gitlancer` workspace dep.
- `Cargo.toml` workspace: `gitlancer` needs `tracing` (for the trait's logging contract); `ora-logging` gets `gitlancer` as a new dependency.
