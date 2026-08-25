# chat-link

Makes assistant Markdown file mentions and expanded tool locations actionable.
This module owns classification and the clickable control; it does not own the
Files viewer, the Diff viewer, or ACP tool collection with line-diff counts.

## Responsibilities

- Parse path-like inline code and Markdown hrefs (`parse.ts`)
- Normalize ANSI terminal output and parse aligned PowerShell tables (`tool-output-table.ts`)
- Build a path-only file and directory artifact index from tool diffs, locations, and glob/search dumps (`artifact-index.ts`)
- Classify a candidate as Diff, Files, Web, or none (`classify.ts`)
- Guard the whole chain against real agent output: `recorded-sessions.test.tsx`
  replays recorded ACP transcripts (`fixtures/`) through the production session
  loader and asserts which prose tokens link and which surface each one opens
- Provide `ChatLinkContext` from the message list and render `ChatFileLink`

## Non-responsibilities

- Bottom-of-turn “edited N files” summaries (`turn-diff-files.ts` / `TurnDiffSummary`)
- Specs `MarkdownDocument`, conversation previews, user messages, thoughts, workflow cards
- Workspace existence probes or a new `openUrl` platform API

## Invariants

- Inline code becomes a link only when it is path-like or its basename hits that turn's typed artifact index.
- Each assistant turn receives a **cumulative** index of tools up to that turn (`collectCumulativeArtifactIndices`). A path that was only read still opens Files on that turn, even if a later turn edits it. Mentions from the edit turn onward open Changes.
- Failed and cancelled tool calls are not indexed.
- When ACP omits `locations`, read tools may still contribute referenced paths from `rawInput` (`filePath`, `path`, `AbsolutePath`, …).
- Navigation uses the index hit’s workspace-relative path, never the raw clicked token when a unique hit exists. A bare token that hits both the listing-root-qualified entry and the bare one (the same visible output also arrives as `rawOutput`) resolves to the workspace-root form instead of cancelling out as ambiguous. A bare filename links when that last segment is unique across **edited and referenced together**. If several hits share the basename, the workspace-root file (`README.md`) still links; same-level nested copies stay plain text. An explicit Markdown file href still attempts Files with the typed string.
- Search/glob/execute dumps that list one artifact per line, including slash-terminated directories and ripgrep-style `path:line:column:text` output (or a `filenames` / `files` array), are indexed even when ACP omitted per-entry `locations`. Explicit files open a preview and explicit directories open Files/Explorer. A recursive listing that prints workspace-relative children (`.claude\commands`) indexes every level, not just the bare top one: a directory has no extension, so the file heuristics alone would drop it. For a recognized listing the visible output is the authority — `rawOutput` repeats the same text and its guesses (a dotfile such as `.claude` reads as a file) never override the listing's verdict, while structured provider kinds still do. Ambiguous entries from a recognized directory-listing operation remain `unknown`; clicking them lists their parent once and follows the returned `WorkspaceEntry.kind` instead of guessing from dots or extensions.
- PowerShell `Mode Name`, `Name Mode`, aligned default tables, and `Name PSIsContainer` (either column order, `True` = directory) provide explicit file/directory evidence. The header rule under any of them (`----   -------------`) is not a row: reading it as one both invents a `----` artifact and suppresses the fallback parse for every real entry below it. Explicit file evidence wins over earlier unknown or directory guesses, including extensionless files such as `install`. Tree markers (`├──`, `└──`) and Markdown list markers are display syntax, not part of the navigation path.
- ANSI-colored PowerShell headers are normalized before column detection. Recognized directory listings may also use aligned multi-column names separated by two or more spaces; those entries stay `unknown` until Files resolves their real kind. Comma/semicolon-separated prose and multi-column plaintext fences are linked token by token, while non-concrete glob summaries such as `README.*.md` remain text.
- Tool dumps keep `whitespace-pre-wrap` so indented JSON, trees, and aligned columns do not collapse. Markdown `a` / `p` / `li` overrides are not re-walked by the parent, so `[src/main.rs](src/main.rs)` cannot nest a file button inside another file button.
- Assistant anchor destinations bypass react-markdown's URL pre-filter so Windows drive paths and `file:` URIs reach this module unchanged; image and other media URLs keep the default filter. Web `http(s)` hrefs keep their original percent-encoding. `javascript:` / `data:` / `vbscript:` / `mailto:` and other non-file schemes (`ssh:`, `ftp:`, …) are inert. File paths decode percent escapes once and accept `:line:column`, `#LlineCcolumn`, query line/column, and `(line N, column N)` suffixes.
- Web links render through `ChatExternalLink` (`../chat-external-link.tsx`), never a bare `<a target="_blank">`. Desktop's main window registers no `on_new_window` hook (only plugin surface webviews do, see `apps/desktop/src-tauri/src/surface/hooks.rs`), so a plain new-window anchor is silently dropped there even though it looks clickable in a browser tab and under jsdom. `ChatExternalLink` routes the click through the platform's `openExternalUrl` command instead, the same one the prompt box uses.
- Absolute ACP paths are stripped with the active Workspace cwd (`getWorkspace` / `resolveWorkspaceCwd`, plus desktop `resolveTaskCwd` when a task exists) before Diff/Files requests. A hit outside that cwd is not a link: suffix-matching it must not open a different relative path inside the worktree. An absolute Markdown href outside the cwd stays unlinked for the same reason (Files rejects rooted paths). Codex may still open absolute paths in View Code; Ora does not, because `readWorkspaceFile` / `readProjectFile` reject rooted paths.
- If a requested diff file is not in the active task patch, navigation falls back to Files. A line missing from a file that **is** in the patch still opens that file in Changes, with no toast.
- If Files cannot read a requested path (including a file the user deleted after the agent read it), the viewer shows the localized missing-path copy, not the raw `Remote Ora request failed` transport message. A new chat `requestId` invalidates the Files query for that path so a deleted file is not shown from cache. An edited path still opens Changes even if the workspace file is gone, because the task patch is independent of the live tree.
- Desktop “File Manager” reveals a file in the **system** file manager (`explorer /select,` on Windows, `open -R` on macOS). It does not open the file with the default editor (often Cursor). Directories still open as folder windows. A missing path is revealed the same way so a deleted file does not fall back to Cursor.
- Changes must keep the requested file selected **and scroll that file's diff section to the top of the viewport**. The first layout after remounting Changes is often 0-height, and virtualized placeholders above the file can shrink after the first jump; do not treat `scrollTo(0)` or a one-shot jump as success. Scroll-position highlighting must not replace a chat-driven `fileRequest` with the first or last file in the patch.
- Switching tasks drops the previous task’s Diff/Files request so a leftover path cannot open in the new worktree.
- The index must not call `diffLines` or read diff text; streaming rebuilds stay cheap.

## Interactions

- `MessageList` provides a per-turn `ChatLinkContext` around each `ResponseTurn` for task or project-only drafts. `ChatView` remounts the list when `taskId` / `projectId` changes so the per-turn artifact cache cannot leak across checkouts.
- `TaskChangesNavigation.openDiff` / `openWorkspaceFile` / `openWorkspaceDirectory` / `openWorkspaceArtifact` in the review layout
- Desktop `locationActions` for Explorer, VS Code, and copying an OS-absolute path. Links reuse the checkout cwd already resolved by `MessageList`; they do not launch one cwd IPC per mounted link. Explorer / VS Code / Copy path are omitted until an OS-absolute cwd is known. A Diff link also offers Preview in Files.
- Shared slash matching in `packages/app-shell/src/lib/workspace-path.ts`

## Appearance

- Clickable file citations use Codex-style path chrome: sky-blue text, no muted code chip, dashed underline on hover. Unlinked inline code keeps the existing chip.
- Web `http(s)` links keep the existing solid primary underline.
