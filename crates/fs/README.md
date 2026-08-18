# ora-fs

`ora-fs` provides read-only, workspace-scoped filesystem primitives shared by Ora runtimes. It deliberately has no HTTP or frontend dependency and returns crate-native errors so transport adapters can choose their own public error and logging policy.

## Guarantees

- Path validation and containment come from `ora-utils::path` (`PortableRelativePath`,
  `CanonicalPathRoot`); this crate applies them to workspace roots and does not maintain local
  path validators. Roots and existing requested paths are canonicalized before containment
  checks, including static symlink escape protection. These path-based checks do not protect
  against a concurrently replaced symlink between validation and use; callers handling actively
  hostile directories need a handle-relative filesystem design.
- File reads are bounded and reject binary or invalid UTF-8 content.
- Search runs through the injected `ora-process` runner, making ripgrep execution replaceable in tests.
- Native watcher events are normalized into workspace-relative changes and can be debounced by the caller.
- The `spec` module discovers Markdown/MDX through the same injected bundled ripgrep, supports explicit ignored sources, and resolves platform-selected directories without allowing workspace escape.

The adapters are documented in [Task Workspace Files](../../docs/task-workspace-files.md) and [Specification management](../../docs/spec-management.md). Tests can inject a `ProcessSpawner` rather than starting ripgrep.
