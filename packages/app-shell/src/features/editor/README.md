# editor

App-shell wrapper around `@ora/editor` for prompt boxes.

## Responsibilities

- Own the Tiptap instance for chat and HITL (`ComposerEditor`).
- Seed and `replaceText` from `documentPlainText` via `markdownToComposerContent`
  so HITL drafts remount as the same nodes rather than leftover `**` / `#`.
  Chat session switches park TipTap JSON (`getJSON` / `replaceDocument`) so
  file/skill/command chips keep their UI and `kind` instead of becoming inline
  code. Text-only restore still rebuilds path backticks and `$skill` tokens
  when possible; slash-command chips and directory `kind` need the parked doc.
  Programmatic hydrate dismisses the `@`/`/` palette so a parked `@…` suffix
  does not reopen the menu. Explorer line selections deliver into the bound
  conversation composer (queued only while that composer is unmounted) and are
  refused (with a warning) when no session/draft/task is selected. A quote the
  composer cannot insert is reported inline rather than dropped silently; it is
  not re-queued, because replaying is what resurrected deleted chips.
  `appendText` inserts parsed blocks at the end so `/command` chips are not
  flattened by a Markdown round-trip. Pastes that include both files and plain text keep the text.
  Typing an opener in front of existing closers (`**`, `==`, `~~`, and the
  rest of the prompt Markdown surface) stays source until a trailing space or
  newline, which then renders that line only. Backspace against contiguous
  converted mark runs restores their Markdown source so deletes edit real characters.
- Map Enter to submit only in a body paragraph. Inside a quote, list, heading,
  or fenced code block, Enter returns to body text in one step. Shift+Enter is
  the newline inside those structures, and also opens a fence from an opener
  line.
- Surface slash/`@` query state from the text immediately before the caret so
  `/` still opens skills/commands after existing prompt text, and `@` drives
  workspace file mentions (chips) owned by the chat composer palette.
- Style kit nodes with Ora CSS variables. Links match the dashboard underline
  and open in the host browser on click. File `@` mentions render as inline
  type-icon + basename refs (Tabler via `WorkspaceFileIcon` / React node view),
  not bordered pills: soft teal basename ink with type-colored icons. Line
  quotes append a muted `L12-34` range; when a snippet was captured,
  `documentPlainText` expands to a citation fence for file-preview quotes, or
  a mini `diff --git` patch for Diff-gutter quotes, while the
  chip stays compact in the composer. File chips with a line range expose
  `data-start-line` / `data-end-line` and a matching `title`. Drag-selecting
  across chips snaps the range onto the chip under the pointer (atoms skip
  native `::selection`) and paints `data-chip-selected` without a React
  re-render. Hovering a file chip swaps its type icon for a remove control in
  the same slot, so the chip keeps its width; the control drops that reference
  from the prompt and is not a tab stop (keyboard removal stays select the chip
  plus Backspace). `==highlight==` is a Typora yellow (`rgb(255, 255, 0)`;
  delimiters hidden). Compact user-message Markdown expands every single
  newline outside fences (including runs of one-character lines). `/` skills
  and `$` commands are mint-wash pills with forest green ink (Cursor-style;
  no neon glow).
- Sent user messages stay `documentPlainText` in the store and render read-only
  via chat `MarkdownDocument` (`density="compact"`). Compact mode expands
  TipTap single newlines outside fences, maps `==highlight==` to `<mark>`, and
  reads quote fences back into chips through `parseComposerFileQuote` so a sent
  quote keeps the composer's compact look instead of unfolding into its source.
  Both surfaces render the chip from `FileRefChipContent` against one unscoped
  stylesheet (`features/file-ref-chip.css`), so the prompt box and the message
  bubble cannot drift apart; the editor scope only adds editing behaviour
  (drag, node selection, `user-select`).
  Future edit remounts `ComposerEditor` on that same string; history rows do
  not keep a TipTap instance.

## Non-responsibilities

- Chat chrome (attachments, model picker, plus menu).
- HITL gate / draft store ownership.
- Spec document editing.
- Read-only rendering of sent user/assistant history (owned by chat
  `MarkdownDocument` / `MarkdownMessage`).

## Performance

The editor is uncontrolled (`shouldRerenderOnTransaction: false`). Parents
re-render on slash/`@`/blankness changes, not on each character of a normal
sentence. HITL drafts subscribe via `onTextChange` and reload through
`markdownToComposerContent` so overlay/embedded remounts keep formatted nodes.
