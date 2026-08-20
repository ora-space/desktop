# chat-link

Makes assistant Markdown file mentions and expanded tool locations actionable.
This module owns classification and the clickable control; it does not own the
Files viewer, the Diff viewer, or ACP tool collection with line-diff counts.

## Responsibilities

- Parse path-like inline code and Markdown hrefs (`parse.ts`)
- Build a **path-only** session artifact index from tool diffs, locations, and glob/search dumps (`artifact-index.ts`)
- Classify a candidate as Diff, Files, Web, or none (`classify.ts`)
- Provide `ChatLinkContext` from the message list and render `ChatFileLink`

## Non-responsibilities

- Bottom-of-turn “edited N files” summaries (`turn-diff-files.ts` / `TurnDiffSummary`)
- Specs `MarkdownDocument`, conversation previews, user messages, thoughts, workflow cards
- Workspace existence probes or a new `openUrl` platform API

## Invariants

- Inline code becomes a link only when it is path-like **and** hits that turn’s index.
- Each assistant turn receives a **cumulative** index of tools up to that turn (`collectCumulativeArtifactIndices`). A path that was only read still opens Files on that turn, even if a later turn edits it. Mentions from the edit turn onward open Changes.
- Failed and cancelled tool calls are not indexed.
- When ACP omits `locations`, read tools may still contribute referenced paths from `rawInput` (`filePath`, `path`, `AbsolutePath`, …).
- Navigation uses the index hit’s workspace-relative path, never the raw clicked token when a unique hit exists. A bare filename links when that last segment is unique across **edited and referenced together**. If several hits share the basename, the workspace-root file (`README.md`) still links; same-level nested copies stay plain text. An explicit Markdown file href still attempts Files with the typed string.
- Search/glob/execute dumps that list one file per line (or a `filenames` / `files` array) are referenced artifacts even when ACP omitted per-file `locations`. Singular keys such as `file` / `path` may also hold an array. `file:` URIs are stripped to filesystem paths. Glob patterns (`**/*.md`) and search-root directories are not indexed. Path-only plaintext fences, markdown list items, and expanded tool dumps use the same classifier.
- Tool dumps keep `whitespace-pre-wrap` so indented JSON, trees, and aligned columns do not collapse. Markdown `a` / `p` / `li` overrides are not re-walked by the parent, so `[src/main.rs](src/main.rs)` cannot nest a file button inside another file button.
- Web `http(s)` hrefs keep their original percent-encoding. `javascript:` / `data:` / `vbscript:` / `mailto:` and other non-file schemes (`ssh:`, `ftp:`, …) are inert. File paths decode `%20` once.
- Absolute ACP paths are stripped with the task cwd (`getWorkspace`, plus desktop `resolveTaskCwd`) before Diff/Files requests. A hit outside that cwd is not a link: suffix-matching it must not open a different relative path inside the worktree. An absolute Markdown href outside the cwd stays unlinked for the same reason (Files rejects rooted paths). Codex may still open absolute paths in View Code; Ora does not, because `readWorkspaceFile` rejects rooted paths.
- If a requested diff file is not in the active task patch, navigation falls back to Files. A line missing from a file that **is** in the patch still opens that file in Changes, with no toast.
- If Files cannot read a requested path (including a file the user deleted after the agent read it), the viewer shows the localized missing-path copy, not the raw `Remote Ora request failed` transport message. A new chat `requestId` invalidates the Files query for that path so a deleted file is not shown from cache. An edited path still opens Changes even if the workspace file is gone, because the task patch is independent of the live tree.
- Desktop “File Manager” reveals a file in the **system** file manager (`explorer /select,` on Windows, `open -R` on macOS). It does not open the file with the default editor (often Cursor). Directories still open as folder windows. A missing path is revealed the same way so a deleted file does not fall back to Cursor.
- Changes must keep the requested file selected **and scroll that file's diff section to the top of the viewport**. The first layout after remounting Changes is often 0-height, and virtualized placeholders above the file can shrink after the first jump; do not treat `scrollTo(0)` or a one-shot jump as success. Scroll-position highlighting must not replace a chat-driven `fileRequest` with the first or last file in the patch.
- Switching tasks drops the previous task’s Diff/Files request so a leftover path cannot open in the new worktree.
- The index must not call `diffLines` or read diff text; streaming rebuilds stay cheap.

## Interactions

- `MessageList` provides a per-turn `ChatLinkContext` around each `ResponseTurn`. `ChatView` remounts the list when `taskId` changes so the per-turn artifact cache cannot leak across tasks.
- `TaskChangesNavigation.openDiff` / `openWorkspaceFile` in the review layout
- Desktop `locationActions` for Explorer, VS Code, and copying an OS-absolute path. The capability is always the cwd + OS-open pair (no `unsupported` variant); Terminal stays hidden because it is a directory opener. An empty `resolveTaskCwd` result keeps the session cwd instead of forcing a blank host path. Explorer / VS Code / Copy path are omitted until an OS-absolute cwd is known so the menu never copies a worktree-relative path. The frontend still passes the file’s OS-absolute path; `open_location` on Desktop reveals that file in the system file manager instead of launching the default association. A Diff (edited) link also offers Preview in Files. A Files (read-only) link has no extra in-app item: left click already opens Files, and View in Changes is omitted because a per-turn index cannot classify the same mention as both Files and edited.
- Shared slash matching in `packages/app-shell/src/lib/workspace-path.ts`

## Appearance

- Clickable file citations use Codex-style path chrome: sky-blue text, no muted code chip, dashed underline on hover. Unlinked inline code keeps the existing chip.
- Web `http(s)` links keep the existing solid primary underline.
