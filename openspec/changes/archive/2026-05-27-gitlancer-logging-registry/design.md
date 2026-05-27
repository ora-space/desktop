## Context

`gitlancer` is a library crate that executes git commands via `CliGitRunner`. It currently has zero observability: no log lines are emitted when commands run, succeed, or fail. Higher-level crates (e.g. `application`) already use `ora-logging` to emit structured tracing events, but because `gitlancer` must remain a lightweight library it cannot force every consumer to carry an `ora-logging` dependency. An opt-in registry pattern lets the application layer wire `ora-logging` into `gitlancer` without coupling the library.

## Goals / Non-Goals

**Goals:**
- Add a `GitlancerLogger` trait and a single-slot global registry inside `gitlancer`.
- Emit structured log events from `CliGitRunner::run()` (command args, duration, exit code, errors) through the registered logger.
- Provide `OraGitlancerLogger` in `ora-logging` that bridges the trait to `tracing` macros so callers need only call `register_gitlancer_logger()` once at startup.
- Guarantee zero overhead (no allocation, no lock contention) on every `run()` call when no logger is registered.

**Non-Goals:**
- Logging from `RecordingGitRunner` (it is a test stub and intentionally opaque).
- Per-instance loggers (the registry is process-wide, matching `tracing`'s global dispatch model).
- Structured spans / trace propagation through git operations (plain events are sufficient for now).

## Decisions

### Decision: `OnceLock<Arc<dyn GitlancerLogger + Send + Sync>>` for the registry

`OnceLock` provides lock-free reads after the first write and is the idiomatic Rust primitive for a write-once global. Wrapping in `Arc` lets the registry slot be read without holding any lock on the hot path. An `Option` slot protected by `Mutex` was considered but rejected: it adds a lock on every `run()` call and allows deregistration, which is not needed.

### Decision: Define `GitlancerLogger` as a trait in `gitlancer`, not as a re-export of `tracing`

Keeping the trait in `gitlancer` means the crate does **not** need to take a `tracing` dependency of its own. The trait has two methods: `log_command` (fired before execution) and `log_result` (fired after). This is the minimal contract for diagnosing command performance and failure. Having `gitlancer` depend on `tracing` directly was considered but rejected because it would force every consumer to pull `tracing` and it would tie `gitlancer`'s logging API to tracing's macro contract.

### Decision: `OraGitlancerLogger` lives in `ora-logging`

`ora-logging` already owns the tracing subscriber initialization and the `ora_info!`/`ora_error!` macros. Putting the bridge implementation there keeps the bridge close to its dependency. An alternative—a separate `gitlancer-logging` crate—was rejected as unnecessary indirection for one struct.

### Decision: `register_gitlancer_logger()` in `ora-logging` is the public entry point

A single zero-argument function in `ora-logging` calls `gitlancer::logging::register(OraGitlancerLogger)`. This makes the happy-path one-liner obvious to callers and hides the cross-crate wiring detail. Callers who want a custom logger can call `gitlancer::logging::register` directly.

## Risks / Trade-offs

- **Risk: `ora-logging` gains a new dependency on `gitlancer`** → Mitigation: `gitlancer` has no dependencies on `ora-logging`, so there is no circular dependency. The dependency is one-directional.
- **Risk: `OnceLock` prevents re-registration** → Accepted trade-off: matching the process-wide tracing subscriber model; tests that need isolation can use `RecordingGitRunner` instead of relying on the logger.
- **Risk: logger interface is too narrow** → If future concerns (e.g., span attachment) are needed, the trait can be extended with default-impl methods to stay backward compatible.
