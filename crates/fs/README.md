# ora-fs

`ora-fs` provides read-only, task-workspace-scoped filesystem primitives for Ora's Web runtime. It deliberately has no HTTP or frontend dependency and returns crate-native errors so transport adapters can choose their own public error and logging policy.

## Guarantees

- Every caller supplies a workspace root, but all user paths must be relative to that root.
- Roots and requested paths are canonicalized before containment checks, including symlink escape protection.
- File reads are bounded and reject binary or invalid UTF-8 content.
- Search runs through the injected `ora-process` runner, making ripgrep execution replaceable in tests.
- Native watcher events are normalized into workspace-relative changes and can be debounced by the caller.

The Web adapter is documented in [Task Workspace Files](../../docs/task-workspace-files.md). Tests can inject a `ProcessSpawner` rather than starting ripgrep.
