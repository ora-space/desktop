## 1. gitlancer: Logger Trait & Registry

- [x] 1.1 Create `crates/gitlancer/src/logging.rs` with `GitlancerLogger` trait (methods: `log_command(&self, cwd: &Path, command: &str)`, `log_result(&self, duration_ms: u64, success: bool, exit_code: Option<i32>)`)
- [x] 1.2 Add `OnceLock<Arc<dyn GitlancerLogger + Send + Sync>>` static and `register(logger: impl GitlancerLogger + 'static)` public function in `logging.rs`
- [x] 1.3 Export `logging` module and `GitlancerLogger` from `crates/gitlancer/src/lib.rs`
- [x] 1.4 Add `tracing` workspace dep to `crates/gitlancer/Cargo.toml` (needed for the bridge; keeps trait clean via no direct tracing usage in gitlancer itself — verify if actually needed or skip if not)

## 2. gitlancer: Instrument CliGitRunner

- [x] 2.1 In `CliGitRunner::run()`, call `logging::get_logger()` (a helper that returns `Option<Arc<dyn GitlancerLogger>>`) at the start of the method
- [x] 2.2 Before spawning, format full command as `"git {args}"` (space-joined) and call `logger.log_command(&command.cwd, &full_command)` (only when logger is Some)
- [x] 2.3 Call `logger.log_result(duration_ms, true, output.status.code())` on success path
- [x] 2.4 Call `logger.log_result(0, false, None)` on spawn-failure error paths (`GitNotFound`, `SpawnFailed`)
- [x] 2.5 Call `logger.log_result(duration_ms, false, code)` on `NonZeroExit` error path

## 3. ora-logging: OraGitlancerLogger Bridge

- [x] 3.1 Add `gitlancer` as a workspace dependency in `crates/logging/Cargo.toml`
- [x] 3.2 Create `crates/logging/src/gitlancer_bridge.rs` with `pub struct OraGitlancerLogger` implementing `gitlancer::logging::GitlancerLogger`
- [x] 3.3 In `log_command`, emit `ora_info!(message = "git command", cwd = %cwd.display(), command)` tracing event
- [x] 3.4 In `log_result`, emit `ora_info!` on success with `duration_ms` and `exit_code`; emit `ora_error!` on failure with `success = false` and `exit_code`
- [x] 3.5 Add `pub fn register_gitlancer_logger()` free function that calls `gitlancer::logging::register(OraGitlancerLogger)`
- [x] 3.6 Export `OraGitlancerLogger` and `register_gitlancer_logger` from `crates/logging/src/lib.rs`

## 4. Tests

- [x] 4.1 In `crates/gitlancer/src/logging.rs` tests: verify `register` is idempotent (second call keeps first logger) using a `MockLogger` with an `Arc<AtomicBool>` hit counter
- [x] 4.2 In `crates/logging/src/gitlancer_bridge.rs` tests: install a test tracing subscriber with `with_recorded_trace_logging`, call `register_gitlancer_logger()`, run a `RecordingGitRunner`-backed git operation, and assert the expected tracing events are emitted

## 5. Cargo Workspace Plumbing

- [x] 5.1 Add `gitlancer` to the workspace `[dependencies]` table in the root `Cargo.toml` if not already present (so `ora-logging` can reference it with `gitlancer = { workspace = true }`)
- [x] 5.2 Run `cargo check --workspace` and fix any compilation errors
- [x] 5.3 Run `cargo fmt --all` and `task test` to confirm all tests pass
