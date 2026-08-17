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

- Inline code becomes a link only when it is path-like **and** hits the session index.
- Edited paths anywhere in the conversation open Changes; read-only referenced paths open Files.
- Navigation uses the index hit’s workspace-relative path, never the raw clicked token when a unique hit exists.
- Absolute ACP paths are stripped with the task cwd before Diff/Files requests.
- The index must not call `diffLines` or read diff text; streaming rebuilds stay cheap.

## Interactions

- `TaskChangesNavigation.openDiff` / `openWorkspaceFile` in the review layout
- Desktop `locationActions` for Explorer, VS Code, and copying an OS-absolute path
- Shared slash matching in `packages/app-shell/src/lib/workspace-path.ts`
