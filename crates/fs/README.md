# ora-fs

`ora-fs` provides read-only, workspace-scoped filesystem primitives shared by Ora runtimes. It deliberately has no HTTP or frontend dependency and returns crate-native errors so transport adapters can choose their own public error and logging policy.

## Guarantees

- Every caller supplies a workspace root, but all user paths must be relative to that root.
- Roots and requested paths are canonicalized before containment checks, including symlink escape protection.
- File reads are bounded and reject binary or invalid UTF-8 content.
- Search runs through the injected `ora-process` runner, making ripgrep execution replaceable in tests.
- Native watcher events are normalized into workspace-relative changes and can be debounced by the caller.
- The `spec` module discovers Markdown/MDX through the same injected bundled ripgrep, supports explicit ignored sources, and resolves platform-selected directories without allowing workspace escape.

The adapters are documented in [Task Workspace Files](../../docs/task-workspace-files.md) and [Specification management](../../docs/spec-management.md). Tests can inject a `ProcessSpawner` rather than starting ripgrep.
