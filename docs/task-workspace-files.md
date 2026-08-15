# Task Workspace Files

Ora's task workspace file feature gives the Web and Desktop frontends a read-only view of the active workspace for a task. It supports directory browsing, bounded text viewing, filename/content search, line selection for chat context, and native file-change refreshes.

## Ownership and flow

The client sends a task id and, where needed, a workspace-relative path. The Web handler asks `ora-backend` to resolve the task's active working directory, then passes that server-owned root to `ora-fs`. The client never supplies a root path, and `ora-fs` does not depend on HTTP or frontend types.

`GET /api/tasks/{taskId}/workspace` exposes that same authoritative root for platform directory-selection UX. Its `branchName` is optional so project-root tasks, non-Git projects, and detached contexts do not invent a branch.

The layers are intentionally narrow:

- `crates/fs` owns path validation, canonical containment checks, file bounds, ripgrep execution, and native watching.
- `apps/web/server/src/service/workspace_file.rs` maps filesystem results to `ora-contracts` values.
- `apps/web/server/src/handlers/workspace_files.rs` owns HTTP extraction, task-root resolution, and watcher setup.
- `apps/web/server/src/handlers/ndjson_stream.rs` owns NDJSON framing, process-shutdown observation, and request lifecycle completion for every Web stream, including workspace watch.
- `apps/desktop/src-tauri/src/workspace_files.rs` maps the same filesystem results to Tauri commands and preserves typed lifecycle errors across IPC.
- `packages/app-shell/src/features/files` owns the file tree, viewer, search UI, cache invalidation, and line-selection handoff to the composer. The same Files panel hosts the Specs sub-view (`workspace-review-files-panel`); Spec catalog/viewer behavior is documented in [Specification management](spec-management.md).

## HTTP operations

| Operation | Request | Response |
| --- | --- | --- |
| `listWorkspaceDirectory` | `POST /api/tasks/{taskId}/files/list`, optional `path` | `ListWorkspaceDirectoryResponse` |
| `readWorkspaceFile` | `POST /api/tasks/{taskId}/files/read`, required `path` | `ReadWorkspaceFileResponse` |
| `searchWorkspace` | `POST /api/tasks/{taskId}/files/search`, `query` and `kind` | `SearchWorkspaceResponse` |
| `watchWorkspace` | `GET /api/tasks/{taskId}/files/watch` | `WorkspaceFileEventBatch` NDJSON stream |

All returned paths are slash-separated and relative to the resolved task workspace. `watchWorkspace` emits `data`, `error`, and `end` frames. Its error frame uses the shared `{ code, params, requestId }` contract, so the frontend can reuse the same remote-error decoder as unary requests. On the Web server the stream also completes when process shutdown begins, so a live Files panel cannot block `Ctrl+C` exit. A terminal error already queued at shutdown is emitted as `error` rather than a successful `end`.

## Desktop operations

The Tauri frontend uses the same contract operation names for unary calls and maps them to `list_workspace_directory`, `read_workspace_file`, and `search_workspace` commands. `watchWorkspace` uses the existing `stream_contract` channel with the operation name preserved, so Desktop and Web share the same file contracts and stream framing.

## Safety and bounds

- Absolute paths, parent traversal, Windows prefix components, and paths that resolve outside the canonical root are rejected.
- Directory listing and file reads are read-only. File reads are limited to 10 MiB and require valid UTF-8 without NUL bytes.
- Content search treats the query as fixed text, not a regular expression. Search is limited to 15 seconds, 8 MiB of process output, and 10,000 results.
- Native watcher events are normalized to workspace-relative paths and coalesced for 100 ms. A rename carries both its previous and current path; ambiguous native events request a full rescan.

## Errors and logging

Workspace filesystem failures stay typed inside `ora-fs` until the Web or Desktop adapter maps them to a transport-neutral `BackendError`. The adapters hide paths and tool diagnostics from public payloads while retaining the Rust source chain for `ErrorReport` logging. Missing paths map to `file_system_path_not_found`; invalid paths, binary files, and invalid UTF-8 map to `invalid_request`; bounded failures keep their payload-size or unprocessable classification; infrastructure failures map to `internal_error`.

The watcher follows the shared stream lifecycle used by ACP session streams. It keeps one canonical request id from response creation through normal end, watcher failure, or cancellation. The handler defers completion logging until the stream ends or emits its typed error frame, preventing an early success event and a later duplicate failure event.

## Contract generation

Rust contracts live in `crates/contracts/src/file_system.rs` and export to `packages/contracts/src/file-system.ts`. Generation-only endpoint metadata in `xtask/src/frontend/namespaces/file_system.rs` marks the watcher as a stream operation, which keeps the generated fetch client aligned with the NDJSON transport. Regenerate the TypeScript contract package with `task export-contracts` after changing these Rust types.
