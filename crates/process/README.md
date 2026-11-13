# ora-process

`ora-process` provides an injectable asynchronous process lifecycle boundary with explicit stdio ownership and process-tree termination.

## Public model

- `ProcessSpec` describes the executable, arguments, working directory, environment overrides, stdio policies, and drop behavior.
- Reaper registration is the default; bounded one-shot commands can explicitly opt out with
  `ProcessSpec::skip_reaper_registration` while retaining local wait and tree-wide kill behavior.
  `TokioProcessSpawner::without_reaper_registration` opts out every child of one spawner, for a
  host that scopes the reaper to one class of its processes.
- On Unix, `ProcessSpec::inherit_descriptor` hands one extra descriptor to that child alone. Every
  other descriptor above the standard streams stays close-on-exec, also under a dropped identity.
- `ProcessStdio` makes piped, inherited, and null streams explicit.
- `ProcessSpawner` abstracts process creation through an associated `ManagedProcess` type for static dispatch and test fakes.
- `ManagedProcess` exposes one-time stdio take operations, id lookup, non-blocking status checks, waiting, and explicit tree-wide kill.
- `TokioProcessSpawner` and `TokioManagedProcess` provide the production Tokio implementation.
- `initialize_reaper` installs Desktop's process-global cleanup sidecar; spawns then register and
  await acknowledgement before returning a managed handle.

On Windows, production children and the cleanup sidecar are created without console windows.
Their stdio contracts remain unchanged, so protocol pipes and inherited diagnostic output still
work without exposing Deno, command-shell, or sidecar terminals beside the desktop UI.

## Lifecycle guarantees

Explicit `kill` requests forceful termination of the entire process tree: a process group on Unix and a Job Object on Windows. It follows start-kill semantics, so success means the OS accepted the request; callers must still await `wait` to reap the direct child and obtain its final status.

Dropping a default managed handle requests termination, while `keep_alive_on_drop` opts out. Runtime teardown cannot guarantee the same tree-wide behavior as explicit `kill`, so callers requiring descendant cleanup must invoke `kill` before dropping the runtime.

When Desktop has initialized `ora-reaper`, every spawned tree is additionally registered over a
private parent-liveness pipe. Direct-child exit notifications discard identifiers only after the
whole Unix process group is gone; Windows descendants remain contained by the reaper's aggregate
Job Object. On Windows the aggregate Job is assigned before each tree's private Job so it remains
the common outer containment boundary for every independently managed process. Explicit Desktop
shutdown and unexpected parent EOF both forcefully terminate every remaining registered tree.
There is an accepted narrow window between OS spawn and synchronous registration during which an
immediate Ora crash can still leave the new process unmanaged.

Piped stdin, stdout, and stderr can each be taken only once. Protocol parsing, buffering, health checks, restart policy, and business meaning intentionally remain in upper layers such as the backend agent runtime.
