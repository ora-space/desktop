## ADDED Requirements

### Requirement: gitlancer SHALL expose an optional logger registry backed by a write-once global slot
The `gitlancer` crate SHALL provide a `GitlancerLogger` trait and a `register` function in a `logging` module. The registry SHALL use a `OnceLock` so that after the first registration all subsequent reads are lock-free. Calling `register` a second time SHALL be a no-op (the first registered logger is retained). When no logger is registered, all logging call sites in `gitlancer` SHALL compile and execute with zero side effects.

#### Scenario: Logger registered before first git command
- **WHEN** a caller invokes `gitlancer::logging::register` with a logger before any git command runs
- **THEN** subsequent git command executions emit events through that logger

#### Scenario: No logger registered
- **WHEN** no logger has been registered
- **THEN** git command executions complete without invoking any logger code and without panicking

#### Scenario: Second registration attempt is ignored
- **WHEN** `gitlancer::logging::register` is called a second time with a different logger
- **THEN** the originally registered logger remains active and the second logger is dropped

### Requirement: CliGitRunner SHALL emit command and result events through the registered logger
`CliGitRunner::run()` SHALL call `GitlancerLogger::log_command` with `GitCommand::cwd` (the directory the git process will be spawned in, not the host program's working directory) and the full command string (e.g. `"git status --porcelain=v2"`) immediately before spawning the process. After the process exits, it SHALL call `GitlancerLogger::log_result` with the elapsed duration in milliseconds, the exit code, and a flag indicating success or failure. On process-spawn failure (`GitExecError::GitNotFound` or `GitExecError::SpawnFailed`), it SHALL call `GitlancerLogger::log_result` with an error marker instead.

#### Scenario: Successful git command is logged
- **WHEN** a git command exits with status 0
- **THEN** `log_command` is called once before execution
- **THEN** `log_result` is called once after execution with `success = true` and the measured duration

#### Scenario: Failed git command is logged
- **WHEN** a git command exits with a non-zero status
- **THEN** `log_command` is called once before execution
- **THEN** `log_result` is called once after execution with `success = false`

#### Scenario: Git binary not found is logged
- **WHEN** the git binary is not found on the system
- **THEN** `log_result` is called with `success = false`

### Requirement: ora-logging SHALL provide OraGitlancerLogger and a register convenience function
The `ora-logging` crate SHALL expose an `OraGitlancerLogger` struct that implements `gitlancer::logging::GitlancerLogger` by forwarding `log_command` to an `ora_info!` tracing event and `log_result` to an `ora_info!` or `ora_error!` event depending on the success flag. The crate SHALL also expose a `register_gitlancer_logger()` free function that constructs an `OraGitlancerLogger` and registers it via `gitlancer::logging::register`.

#### Scenario: Convenience registration wires tracing into gitlancer
- **WHEN** a caller invokes `ora_logging::register_gitlancer_logger()`
- **THEN** subsequent git command executions emit structured tracing events via the installed subscriber

#### Scenario: log_command emits an info event with cwd and full command string
- **WHEN** `OraGitlancerLogger::log_command` is called with the git command's working directory and a full command string
- **THEN** a tracing info event is emitted containing both `cwd` (the git process directory) and the command string

#### Scenario: log_result emits an error event on failure
- **WHEN** `OraGitlancerLogger::log_result` is called with `success = false`
- **THEN** a tracing error event is emitted
