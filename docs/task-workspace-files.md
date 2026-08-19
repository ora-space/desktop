# Task Workspace Files

Desktop's task workspace file feature provides a read-only view of the active
task workspace. It supports directory browsing, bounded text viewing,
filename/content search, line selection for chat context, and native file-change
refreshes.

## Ownership and flow

The client sends a task id and, where needed, a workspace-relative path. Tauri
commands ask the Desktop filesystem service to resolve the task's authoritative
working directory and never accept a caller-provided root. `ora-utils` owns
portable path validation and canonical containment checks; `ora-fs` applies them
to workspace roots and owns file bounds, ripgrep execution, and native watching.

The layers remain narrow:

- `apps/desktop/src-tauri/src/workspace_files.rs` maps filesystem results to
  contract values and preserves typed lifecycle errors across IPC.
- `apps/desktop/src-tauri/src/commands.rs` owns Tauri extraction, task-root
  resolution, and the command/channel boundary.
- `packages/app-shell/src/features/files` owns the file tree, viewer, search UI,
  cache invalidation, and line-selection handoff to the composer. The same
  Files panel hosts the Specs sub-view; see [Specification management](spec-management.md).
  Chat inline artifact links open this panel through `openWorkspaceFile` and a
  `WorkspaceFileRequest` (`path` + `requestId` + optional line/column) so a
  second click on the same file still applies. The view strips a task-cwd
  prefix from absolute ACP paths, expands ancestor directories so the tree
  shows the file, and selects the optional line so the viewer can highlight
  and scroll to it. A hit outside the task cwd is not opened as a
  worktree-relative path (chat leaves those mentions unlinked). Missing files,
  including a path the user deleted after the agent read it, show the
  localized missing-path copy rather than the raw transport error. A new
  chat `requestId` invalidates the Files query for that path so a second
  open re-reads disk instead of keeping cached content. The backend still
  rejects rooted paths. Desktop File Manager reveals the OS-absolute path
  in the system file manager instead of launching Cursor.

## Desktop operations

| Operation                | Request                            | Delivery                        |
| ------------------------ | ---------------------------------- | ------------------------------- |
| `listWorkspaceDirectory` | `taskId`, optional relative `path` | `list_workspace_directory`      |
| `readWorkspaceFile`      | `taskId`, relative `path`          | `read_workspace_file`           |
| `searchWorkspace`        | task id and bounded search query   | `search_workspace`              |
| `watchWorkspace`         | `taskId`                           | `stream_contract` Tauri channel |

All returned paths are slash-separated and relative to the resolved task
workspace. `watchWorkspace` emits `data`, `error`, and `end` frames. Its error
frame uses the shared `{ code, params, requestId }` contract, so the frontend
uses the same decoder as unary commands. A terminal error already queued during
shutdown is emitted as `error` rather than a successful `end`.

## Safety and UI behavior

Workspace roots are resolved from persisted task identity. Paths are validated
as relative paths, canonicalized before containment checks, and bounded before
reads or searches. The filesystem service is read-only and watcher changes are
cache-invalidating batches rather than an event log.

The Files panel opens Explorer by default for a task and keeps Specs as a
dedicated read-only sub-view. Search and file reads are cancellable through the
injected contracts client, and a mounted watcher is stopped when the panel is
unmounted or its task changes.

Rust contracts live in `crates/contracts/src/file_system.rs` and export to
`packages/contracts/src/file-system.ts`. The endpoint catalog in
`xtask/src/frontend/namespaces/file_system.rs` marks the watcher as a stream
operation. Regenerate the TypeScript contract package with
`task export-contracts` after changing these Rust types.
