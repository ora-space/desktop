# diff

Task Changes panel: parsed worktree patches, file tree, and git commit/push actions.

## Responsibilities

- Render unified/split diffs for a task scope (`branch` / staged / unstaged / commit).
- Collapse distant unchanged context and restore it on demand.
- Jump from chat file links to a new-side line inside the active patch.
- Quote visible diff lines into the chat composer via gutter `+` (click) or
  gutter/`+` drag-release. Clicking a line number pins a highlight on that
  side only (shift-click extends within the same side); Ctrl/Cmd+Enter on a
  focused line number quotes the pinned range, which is the keyboard route to
  the `+` (the `+` itself stays out of the tab order so a long file does not
  add one tab stop per line). Split view keeps both columns numbered; only
  unified collapses context rows to a single number. New-side insert/normal
  rows use the new path; pure deletes use the old path. Drag uses min–max line
  fill (including collapsed content, matching Cursor). A visual Diff range
  that crosses delete / insert / context stays **one chip** (basename +
  `L12-14`); add vs delete is only in the agent payload (`+/-/ `). A drag
  that spans collapsed hunks also stays one chip. File-preview gaps still
  split chips. Collapsed blocks stay non-quotable as click targets until
  expanded.
  Plus is a CSS `+` on the button (no per-line SVG). Unified view hosts every
  `+` in the old (left) gutter so delete, insert, and context share one
  vertical track; a drag that starts on a delete continues through inserts
  below (visual row order). Split view keeps each `+` in its own column and
  locks a drag to that side. It shows for
  the whole hovered change (react-diff-view line hover on code or gutter);
  drag starts from the gutter cell or the `+` and paints selection
  imperatively on that side's gutter and code cells so a split view never
  tints the opposite column.
  On send the chip stays basename + `L12-14`, but the agent payload is a mini
  `diff --git` patch (`--- a/` / `+++ b/` / `@@` / unified `+/-/ ` body, with
  a hunk header `quoted from git diff`, plus `(old|new side)` when the chip
  is a single side) so a follow-up comment is clearly about the existing git
  change, not current file contents.
- Own commit/push UI for the same task worktree.

## Non-responsibilities

- Composer chip rendering or TipTap document ownership (handoff goes through
  `addComposerFileSelection` → `composer-file-context-store`).
- Workspace file browsing (Files / Specs panels).
- Diff comment threads (removed).
