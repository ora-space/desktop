# composer

Tiptap preset and plain-text helpers for prompt boxes (chat composer, HITL).

## Responsibilities

- Provide `createComposerExtensions()` with the dashboard SimpleEditor WYSIWYG
  model: kit-styled exclusive links, file chips, and inline skill/command
  mentions.
- Convert between Tiptap documents and newline-delimited Markdown-ish plain
  text without HTML parsing (headings 1–6, lists, quotes, fences, marks, chips).
  Typed Markdown uses Tiptap input rules when the closer is typed at the
  caret. After the opener is added in front of existing closers (`**`, `*`, `~~`,
  `==`, backticks, `__`, `_`, `***`), a trailing space or newline re-parses
  **that** still-plain line only — other leftover lines stay source until they
  are confirmed. Converting on the first `*` of `*bold**` would steal
  italic. Backspace against contiguous converted mark runs (not a heading, and
  not plain text typed after the marks) restores their Markdown source so further
  deletes remove real characters. Typing at the start of a converted mark stays
  inside it; typing after it is body text. Pasted Markdown and restored
  `documentPlainText` drafts parse into the same nodes so `#` / `**` /
  `- [ ]` still render as structure. Path-like backticks (slash paths, known
  source extensions, dotfiles) and `$skill` tokens become chips on that text
  path; versions (`v1.0`), globs (`*.ts`), slash-command chips, and directory
  `kind` need TipTap JSON parking (chat composer) for a lossless round-trip.
  Ranged quotes may carry a `snippet` attr so send expands to a
  `start:end:path` citation fence; Diff-gutter quotes also set `origin: "diff"`
  and expand to a mini `diff --git` patch with unified markers. A mixed
  add/delete range is still one chip; `diffSide` is omitted and the body
  carries `+/-/ `.
  Path-only `@` mentions stay backtick paths.
  Inline code that contains backticks, and fenced blocks that contain a ` ``` `
  line, serialize with a longer CommonMark fence so parse cannot close early.
  `[label](javascript:…)` / `data:` / `vbscript:` / `file:` hrefs stay literal
  text instead of becoming a link mark. Link titles escape `\` then `"` so a
  quoted title round-trips. Raw strings that must stay literal (`<`
  as text) use `plainTextToComposerContent`. `[ ] ` at the start of a paragraph
  or bullet becomes a task item (not a checklist nested under a disc). Marks
  reuse the kit Bold / Italic / Strike / Code / Highlight / Link / Underline
  extensions. Kit input rules require a leading ASCII space, which misses
  `你好**等等**`; composer keeps `markInputRule` and only relaxes flanking so
  CJK can sit against `*` / `**` / `***` / `~~` / `==`. Underline is Mod-u
  (Markdown has no `__underline__` syntax; `__` is bold). `documentPlainText`
  therefore drops underline; park TipTap JSON to keep it across session switches.
- Shift+Enter is always a newline inside the current structure (code, quote,
  list, heading). Enter leaves that structure and returns to body text, or
  sends from a body paragraph. An empty heading or list item is empty only
  when it has no chips (content size, not `textContent`). Shift+Enter on an
  empty quote lifts the quote instead of inserting another paragraph inside it.
  A trailing space after a fence info string also
  opens the fence.
- Prompt links open on pointerdown (not mouseup/`click`) so the first press
  leaves the editor instead of placing a caret. Desktop still routes through
  the host browser command; this preset only calls `window.open` when no
  platform adapter intercepts the event.

## Capability minimum set

`COMPOSER_CAPABILITIES` is the supported surface. Anything else in the full-page
kit (images, TOC, video, alignment) is out of scope.

| Layer  | Nodes / marks                                                                                                                                               |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Blocks | paragraph, heading 1–6, blockquote, fenced code, bullet/ordered/task lists, `---`                                                                           |
| Marks  | bold, italic, underline, strike, inline code, highlight, link                                                                                               |
| Chips  | `composerFile` (chip), `promptToken` (mention); drag-select snaps onto the chip under the pointer and paints `data-chip-selected` without a React re-render |

Replace a slot with `features: { link: false }` or `features: { link: MyLink }`
and append extras via `extraExtensions`. Rendering stays CSS in the product shell
so the preset does not pull dashboard `--tt-*` SCSS.

## GFM subset

Typed and pasted inline marks follow GitHub Flavored Markdown flanking: the
opener is not followed by a space, the closer is not preceded by a space, and
`_` does not fire inside `snake_case`. `==highlight==` drops the delimiters and
renders as Typora-style yellow (`<mark>`). `***bold-italic***` applies both
marks. Adjacent `**bold***em*` / `*em***bold**` share the middle `***` (bold
close plus italic open, or the reverse). A line that is only `***` is still a
rule. Nested `>` quotes and lists
inside a quote parse on paste. `[label](url "title")` keeps the title.
Mod-u underline is a product extra, not GFM.

Out of this subset on purpose: tables, images, footnotes, HTML blocks, nested
list indent on paste (Tab still nests while typing), and setext headings.

## Non-responsibilities

- Product keymap (Enter to send), attachments, `@` / `/` menus, theming.
- Full-page document chrome (TOC, images, diffs).
- CommonMark extras the prompt box does not own: tables, images, footnotes,
  HTML blocks.
- Dashboard `--tt-*` SCSS; prompt surfaces style against Ora tokens instead.
