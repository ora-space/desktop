# chat-link

Makes assistant Markdown file mentions and expanded tool locations actionable.
This module owns classification and the clickable control; it does not own the
Files viewer, the Diff viewer, or ACP tool collection with line-diff counts.

## Responsibilities

- Parse path-like inline code and Markdown hrefs (`parse.ts`)
- Build a **path-only** session artifact index from tool diffs and locations (`artifact-index.ts`)
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
- Navigation uses the index hit’s workspace-relative path, never the raw clicked token when a unique hit exists. A bare filename must not replace a nested index path.
- Absolute ACP paths are stripped with the task cwd (`getWorkspace`, plus desktop `resolveTaskCwd`) before Diff/Files requests. A hit outside that cwd is not a link: suffix-matching it must not open a different relative path inside the worktree.
- If a requested diff file is not in the active task patch, navigation falls back to Files. A line missing from a file that **is** in the patch still opens that file in Changes, with no toast.
- If Files cannot read a requested path, the viewer shows the localized missing-path copy, not the raw `Remote Ora request failed` transport message.
- Changes must keep the requested file selected **and scroll that file's diff section to the top of the viewport**. The first layout after remounting Changes is often 0-height, and virtualized placeholders above the file can shrink after the first jump; do not treat `scrollTo(0)` or a one-shot jump as success. Scroll-position highlighting must not replace a chat-driven `fileRequest` with the first or last file in the patch.
- Switching tasks drops the previous task’s Diff/Files request so a leftover path cannot open in the new worktree.
- The index must not call `diffLines` or read diff text; streaming rebuilds stay cheap.

## Interactions

- `MessageList` provides a per-turn `ChatLinkContext` around each `ResponseTurn`
- `TaskChangesNavigation.openDiff` / `openWorkspaceFile` in the review layout
- Desktop `locationActions` for Explorer, VS Code, and copying an OS-absolute path. The capability is always the cwd + OS-open pair (no `unsupported` variant); Terminal stays hidden because it is a directory opener. An empty `resolveTaskCwd` result keeps the session cwd instead of forcing a blank host path.
- Shared slash matching in `packages/app-shell/src/lib/workspace-path.ts`

## Appearance

- Clickable file citations use Codex-style path chrome: sky-blue text, no muted code chip, dashed underline on hover. Unlinked inline code keeps the existing chip.
- Web `http(s)` links keep the existing solid primary underline.
