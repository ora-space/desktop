# Task Workspace Files

Desktop's task workspace file feature provides directory browsing, bounded
text viewing, filename/content search, line quoting into chat via gutter `+`
(click or drag, or Ctrl/Cmd+Enter on a focused line number), native
file-change refreshes, and bounded create of a new empty file or directory
from the explorer tree, plus cut/copy/paste, rename, and confirmed delete.
The viewer itself stays read-only.

## Ownership and flow

The client sends a task id and, where needed, a workspace-relative path. Tauri
commands ask the Desktop filesystem service to resolve the task's authoritative
working directory and never accept a caller-provided root. `ora-utils` owns
portable path validation and canonical containment checks; `ora-fs` applies them
to workspace roots and owns file bounds, ripgrep execution, and native watching.

The layers remain narrow:

- `apps/desktop/src-tauri/src/workspace_files.rs` maps filesystem results to
  contract values and preserves typed lifecycle errors across IPC.
- `apps/desktop/src-tauri/src/commands/files.rs` owns Tauri extraction, task-root
  resolution, and the command/channel boundary.
- `packages/app-shell/src/features/files` owns the file tree, viewer, search UI,
  cache invalidation, and gutter `+` line quotes into the composer. A quote
  stays a compact chip on both sides of send: the prompt carries a backtick
  `path:range` reference (the agent reads the body itself), and chat history
  reads that back into the same chip instead of replaying source. Diff-gutter
  quotes are the exception — they expand to a mini `diff --git` patch because
  the change is not yet on disk.
  Chat inline artifact links open this panel through `openWorkspaceFile` and a
  `WorkspaceFileRequest` (`path` + `requestId` + optional
  `FileNavigationLocation` fields: line/column/endLine) so a second click on
  the same file still applies. The view strips a task-cwd prefix from absolute
  ACP paths, expands ancestor directories so the tree shows the file, and
  selects the optional line or inclusive start–end range so the viewer can
  highlight and scroll to it. Citation ranges use the same `--quote-tint` wash
  as a pinned quote (including the gutter); search matches keep amber plus
  `<mark>`. A later click outside a citation range dismisses the wash and the
  header `:start-end` label until the next jump. Search matches stay until the
  next result. Gutter `+` and other buttons do not dismiss. A hit outside the
  task cwd is not opened as a
  worktree-relative path (chat leaves those mentions unlinked). Missing files,
  including a path the user deleted after the agent read it, show the
  localized missing-path copy rather than the raw transport error. A new
  chat `requestId` invalidates the Files query for that path so a second
  open re-reads disk instead of keeping cached content. The backend still
  rejects rooted paths. Desktop File Manager reveals the OS-absolute path
  in the system file manager instead of launching Cursor.

## Project checkout files (draft / no task)

When a chat has a selected project but no task yet (blank or draft composer), Files panel Explorer/Search and `@` file mentions resolve against the **project checkout root** instead of a task worktree. The same main Workspace supplies the working directory used for model discovery and first-send session creation.

These operations reuse the same `ora-fs` list/search/read bounds and relative-path rules as the task APIs. Live watching uses `watchProject` against the project checkout the same way `watchWorkspace` watches a task worktree. When both `taskId` and `projectId` are present, Files and the composer prefer the task worktree so linked-worktree checkouts stay authoritative.

## Desktop operations

| Operation                | Request                                 | Delivery                        |
| ------------------------ | --------------------------------------- | ------------------------------- |
| `listWorkspaceDirectory` | `taskId`, optional relative `path`      | `list_workspace_directory`      |
| `readWorkspaceFile`      | `taskId`, relative `path`               | `read_workspace_file`           |
| `searchWorkspace`        | task id and bounded search query        | `search_workspace`              |
| `watchWorkspace`         | `taskId`                                | `stream_contract` Tauri channel |
| `listProjectDirectory`   | `projectId`, optional relative `path`   | `list_project_directory`        |
| `readProjectFile`        | `projectId`, relative `path`            | `read_project_file`             |
| `searchProject`          | project id and bounded search query     | `search_project`                |
| `watchProject`           | `projectId`                             | `stream_contract` Tauri channel |
| `createWorkspaceEntry`   | `taskId`, relative `path`, `kind`       | `create_workspace_entry`        |
| `createProjectEntry`     | `projectId`, relative `path`, `kind`    | `create_project_entry`          |
| `copyWorkspaceEntry`     | `taskId`, `from`, destination `path`    | `copy_workspace_entry`          |
| `copyProjectEntry`       | `projectId`, `from`, destination `path` | `copy_project_entry`            |
| `moveWorkspaceEntry`     | `taskId`, `from`, destination `path`    | `move_workspace_entry`          |
| `moveProjectEntry`       | `projectId`, `from`, destination `path` | `move_project_entry`            |
| `deleteWorkspaceEntry`   | `taskId`, relative `path`               | `delete_workspace_entry`        |
| `deleteProjectEntry`     | `projectId`, relative `path`            | `delete_project_entry`          |

Create refuses to replace an existing path (`file_system_path_already_exists`).
The parent directory must already exist. Explorer context menus create inside a
folder, or in the parent of a file (Cursor's Files panel). Blank tree space
creates at the checkout root. Cut / Copy / Paste operate on the entry itself
(paste into a folder, or into a file's parent; a same-folder copy gets a
`name copy` suffix). Rename is an inline move of the basename. Delete asks for
confirmation, then permanently removes the path (files, folders, and
unfollowed symlinks).
Copy Path / Copy Relative Path / Reveal in File Manager reuse
`joinOsAbsolutePath` and `locationActions.open("explorer", …)`.

All returned paths are slash-separated and relative to the resolved checkout.
`watchWorkspace` and `watchProject` emit `data`, `error`, and `end` frames. Their error
frame uses the shared `{ code, params, requestId }` contract, so the frontend
uses the same decoder as unary commands. A terminal error already queued during
shutdown is emitted as `error` rather than a successful `end`.

## Safety and UI behavior

Workspace roots are resolved from persisted task or project identity. Paths are
validated as relative paths, canonicalized before containment checks, and
bounded before reads, searches, or creates. Create is a contained write of a
new empty file (`create_new`) or directory (`create_dir`); copy, move, and
delete refuse to escape the checkout and do not follow symbolic links. Delete
asks for confirmation in the explorer, then permanently removes the path. Watcher
changes are cache-invalidating batches rather than an event log.

The viewer keeps mounting every row for files up to 400 lines. Larger files
render only the window around the viewport (plus overscan) so a multi-megabyte
preview costs a bounded DOM instead of blocking the session switches that
remount the panel; files above ~512 KiB of characters additionally defer their
content passes until after the first paint and show a loading overlay, so the
session switch itself never waits on the preview; the scrollable width comes
from a monospace column estimate of the widest line, pinned ranges re-apply
declaratively as rows mount, and syntax highlighting stays capped at 512 KiB
as before.

The Files panel opens Explorer by default for both task and project review
contexts. Search and file
reads are cancellable through the injected contracts client. A mounted
watcher is stopped when the panel is unmounted or its Files scope (task vs
project) changes.

The task Changes (diff) panel applies the same "render the session, then load
the content" discipline. It splits the backend's unified patch back into
per-file slices and caches the parsed `FileData` per slice, so a live-sync
invalidation that rewrites only a few files keeps every unchanged file's
`memo` comparison intact. Patches above 512 KiB of characters defer parsing
past the first paint (a loading state shows instead of a "no changes" message
until the slice parse lands). Diffs that are large either by file count, by
total changed lines, or by a single file big enough to row-window switch to a
single-file focus body — the right-hand file tree drives the selection instead
of scroll-spy — so a large change-set never pays for mounting every file in one
scroll. Each file still renders with the native `react-diff-view` styling
(unified and split) so the gutter, tint, and collapse stay identical to the
plain path. A file above 400 changed lines in the focus body is additionally
chunk-windowed: it is split into fixed-size native `react-diff-view` hunks and
only the visible chunk window is mounted (via `@tanstack/react-virtual`), so a
multi-thousand-line file scrolls with bounded DOM while every mounted row is a
real `Diff`/`Hunk`. Windowing lives inside the file component itself, so all
interaction behavior — chat citation jump-to-line, line quoting, collapse and
expand — keeps working: a jump scrolls the virtualizer to the cited chunk, then
to the highlighted row. The body is chosen automatically from the diff size: a
large change-set (many files, many changed lines, or one file big enough to
row-window) uses the single-file focus body, otherwise the continuous scroll
body. There is no manual toggle, because the scroll body mounts every file as
one native table and cannot window per-file — a big file there would stall,
while the focus body's bounded-DOM rendering keeps it fast.

Rust contracts live in `crates/contracts/src/file_system.rs` and export to
`packages/contracts/src/file-system.ts`. The endpoint catalog in
`xtask/src/frontend/namespaces/file_system.rs` marks the watcher as a stream
operation. Regenerate the TypeScript contract package with
`task export-contracts` after changing these Rust types.
