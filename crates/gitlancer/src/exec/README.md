# Gitlancer Execution Module

This module is the process boundary for prepared Git CLI operations.

## Execution contract

- `GitCommand` carries the working directory, arguments, `GitEnv`, and a `GitIntent` classification.
- `GitEnv::automation_defaults` disables terminal prompts, fixes language to `C`, and disables paging so agent-driven commands cannot block on interactive UI or localized output.
- `GitRunner` lets the typed use-case layer execute commands through static dispatch.
- `CliGitRunner` invokes the system `git` binary, records duration, emits optional command telemetry, and returns normalized `GitOutput`.
- `RecordingGitRunner` provides a non-executing boundary for command-construction tests.

A missing executable, spawn failure, or non-zero exit is a `GitExecError`. Non-zero output retains exit code, arguments, stdout, and stderr for upper-layer diagnostics.

This module does not validate repository semantics, assemble use-case-specific arguments, parse Git output, or decide whether a mutating/network command is authorized. Those concerns belong to the domain, `git`, `parse`, and application policy layers respectively.

See the [gitlancer overview](../../README.md) and [Gitlancer Architecture](../../../../docs/gitlancer-architecture.md).
