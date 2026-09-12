# ora-fs

`ora-fs` provides workspace-scoped filesystem primitives shared by Ora runtimes:
contained listing, bounded reads, search, watching, create of a missing empty
file or directory, and contained copy, move, and delete. It deliberately has no HTTP or frontend dependency and returns
crate-native errors so transport adapters can choose their own public error and
logging policy.

## Guarantees

- Path validation and containment come from `ora-utils::path` (`PortableRelativePath`,
  `CanonicalPathRoot`); this crate applies them to workspace roots and does not maintain local
  path validators. Roots and existing requested paths are canonicalized before containment
  checks, including static symlink escape protection. These path-based checks do not protect
  against a concurrently replaced symlink between validation and use; callers handling actively
  hostile directories need a handle-relative filesystem design.
- File reads are bounded and reject binary or invalid UTF-8 content.
- Create joins the new name onto an already-contained parent and uses
  `create_new` / `create_dir`, so an existing path is `AlreadyExists` rather than
  a truncate.
- Copy and move join onto an already-contained parent, refuse to replace an
  existing path, and refuse to place a directory inside itself. Directory copy
  skips symbolic links instead of following them.
- Delete unlinks a contained file, directory tree, or symlink without following
  links, and refuses the workspace root.
- Search runs through the injected `ora-process` runner, making ripgrep execution replaceable in tests.
- Native watcher events are normalized into workspace-relative changes and can be debounced by the caller.

The adapters are documented in [Task Workspace Files](../../docs/task-workspace-files.md). Tests can inject a `ProcessSpawner` rather than starting ripgrep.
