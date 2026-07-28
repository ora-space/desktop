# ora-process

`ora-process` provides an injectable asynchronous process lifecycle boundary with explicit stdio ownership and process-tree termination.

## Public model

- `ProcessSpec` describes the executable, arguments, working directory, environment overrides, stdio policies, and drop behavior.
- `ProcessStdio` makes piped, inherited, and null streams explicit.
- `ProcessSpawner` abstracts process creation through an associated `ManagedProcess` type for static dispatch and test fakes.
- `ManagedProcess` exposes one-time stdio take operations, id lookup, non-blocking status checks, waiting, and explicit tree-wide kill.
- `TokioProcessSpawner` and `TokioManagedProcess` provide the production Tokio implementation.

## Lifecycle guarantees

Explicit `kill` requests forceful termination of the entire process tree: a process group on Unix and a Job Object on Windows. It follows start-kill semantics, so success means the OS accepted the request; callers must still await `wait` to reap the direct child and obtain its final status.

Dropping a default managed handle requests termination, while `keep_alive_on_drop` opts out. Runtime teardown cannot guarantee the same tree-wide behavior as explicit `kill`, so callers requiring descendant cleanup must invoke `kill` before dropping the runtime.

Piped stdin, stdout, and stderr can each be taken only once. Protocol parsing, buffering, health checks, restart policy, and business meaning intentionally remain in upper layers such as the backend agent runtime.
