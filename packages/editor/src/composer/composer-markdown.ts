import type { JSONContent } from "@tiptap/core";
import { Extension } from "@tiptap/core";
import { Plugin, PluginKey } from "@tiptap/pm/state";
import { isDangerousComposerHref } from "./composer-link";
import { plainTextToComposerContent } from "./composer-plain-text";

type MarkSpec = { type: string; attrs?: Record<string, string | null> };

const HEADING = /^(#{1,6})(?:\s+|$)(.*)$/;
const TASK = /^(\s*)[-*+]\s+\[([ xX])\]\s+(.*)$/;
const BULLET = /^(\s*)[-*+]\s+(.*)$/;
const ORDERED = /^(\s*)(\d+)\.\s+(.*)$/;
const FENCE = /^(```+)([^\s`]*)$/;
const RULE = /^(?:---|\*\*\*|___)\s*$/;

/** CommonMark: a closing fence is a line of backticks at least as long as the opener. */
function isFenceClose(line: string, openLen: number): boolean {
  const match = /^(```+)$/.exec(line);
  return match !== null && (match[1]?.length ?? 0) >= openLen;
}
const HTML_CLIPBOARD =
  /<(p|div|h[1-6]|ul|ol|li|pre|blockquote|strong|em|a|code|hr)\b/i;
const INLINE_MARKDOWN =
  /(\*\*\*(?!\s)[^*]+(?<!\s)\*\*\*|\*\*(?!\s)[^*]+(?<!\s)\*\*|__(?!\s)[^_]+(?<!\s)__|(?<!\*)\*(?![*\s])[^*]+(?<!\s)\*(?!\*)|(?<![A-Za-z0-9_])_(?![_\s])[^_]+(?<!\s)_(?![A-Za-z0-9_])|~~(?!\s)[^~]+(?<!\s)~~|==(?!\s)[^=]+(?<!\s)==|`[^`]+`|\[[^\]]+\]\([^)]+\))/;
/** Prompt pastes are small; cap quote recursion so `>>>>>>>>>>…` cannot blow the stack. */
const MAX_QUOTE_DEPTH = 32;

/**
 * True when clipboard text uses the composer's Markdown surface, so paste
 * should build nodes instead of dumping literal `#` / `**` into a paragraph.
 */
export function looksLikeComposerMarkdown(text: string): boolean {
  if (text.length === 0) {
    return false;
  }
  return (
    /^(#{1,6}\s|\s*[-*+]\s|\s*\d+\.\s|>\s|```|---|___|\*\*\*)/m.test(text) ||
    INLINE_MARKDOWN.test(text)
  );
}

/**
 * Turns Markdown that the prompt box can represent into Tiptap JSON.
 * Inverse of `documentPlainText` for paste and HITL draft restore.
 * HTML tags stay text so `<script>` cannot become markup.
 * Backtick spans that look like workspace paths become `composerFile` chips;
 * `$name` skill tokens become `promptToken` nodes. Slash-command chips and
 * directory `kind` need TipTap JSON parking for a lossless session switch.
 */
export function markdownToComposerContent(text: string): JSONContent {
  const blocks = parseBlocks(text.replace(/\r\n/g, "\n").split("\n"));
  return {
    type: "doc",
    content: blocks.length === 0 ? [{ type: "paragraph" }] : blocks,
  };
}

/**
 * Pastes Markdown as composer nodes when the clipboard is plain text.
 * HTML copies (browser/editor) keep ProseMirror's default path.
 */
export const ComposerMarkdownPaste = Extension.create({
  name: "composerMarkdownPaste",
  priority: 50,

  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: new PluginKey("composerMarkdownPaste"),
        props: {
          handlePaste: (_view, event) => {
            if (event.clipboardData === null) {
              return false;
            }
            if (event.clipboardData.files.length > 0) {
              return false;
            }
            const html = event.clipboardData.getData("text/html");
            if (html.length > 0 && HTML_CLIPBOARD.test(html)) {
              return false;
            }
            const text = event.clipboardData.getData("text/plain");
            const isMarkdown = looksLikeComposerMarkdown(text);
            if (!isMarkdown && !/[\r\n]/.test(text)) {
              return false;
            }
            const doc = isMarkdown
              ? markdownToComposerContent(text)
              : plainTextToComposerContent(text.replace(/\r\n?/g, "\n"));
            return this.editor.commands.insertContent(doc.content ?? []);
          },
        },
      }),
    ];
  },
});

function parseBlocks(lines: string[], quoteDepth = 0): JSONContent[] {
  const blocks: JSONContent[] = [];
  let index = 0;

  while (index < lines.length) {
    const line = lines[index];
    if (line === undefined) {
      break;
    }

    const fence = FENCE.exec(line);
    if (fence !== null) {
      const openTicks = fence[1] ?? "```";
      const language = fence[2] === "" ? null : (fence[2] ?? null);
      const body: string[] = [];
      index += 1;
      while (
        index < lines.length &&
        !isFenceClose(lines[index] ?? "", openTicks.length)
      ) {
        body.push(lines[index] ?? "");
        index += 1;
      }
      if (
        index < lines.length &&
        isFenceClose(lines[index] ?? "", openTicks.length)
      ) {
        index += 1;
      }
      blocks.push(codeBlock(language, body.join("\n")));
      continue;
    }

    if (RULE.test(line) && line.trim().length > 0) {
      blocks.push({ type: "horizontalRule" });
      index += 1;
      continue;
    }

    const heading = HEADING.exec(line);
    if (heading !== null) {
      blocks.push({
        type: "heading",
        attrs: { level: heading[1].length },
        content: parseInline(heading[2] ?? ""),
      });
      index += 1;
      continue;
    }

    if (line.startsWith(">")) {
      if (quoteDepth >= MAX_QUOTE_DEPTH) {
        blocks.push(paragraph(line));
        index += 1;
        continue;
      }
      const quoted = parseQuoteRun(lines, index, quoteDepth);
      blocks.push({
        type: "blockquote",
        content:
          quoted.inner.length === 0 ? [{ type: "paragraph" }] : quoted.inner,
      });
      index = quoted.next;
      continue;
    }

    if (TASK.test(line) || BULLET.test(line) || ORDERED.test(line)) {
      const { node, next } = parseList(lines, index);
      blocks.push(node);
      index = next;
      continue;
    }

    if (line.length === 0) {
      index += 1;
      continue;
    }

    blocks.push(paragraph(line));
    index += 1;
  }

  return blocks;
}

/**
 * Consecutive `>` lines are one quote. Peel a single marker and parse the
 * rest as blocks so `> >` nests and `> - item` becomes a list inside.
 */
function parseQuoteRun(
  lines: string[],
  start: number,
  quoteDepth: number,
): { inner: JSONContent[]; next: number } {
  const peeled: string[] = [];
  let index = start;
  while (index < lines.length) {
    const line = lines[index];
    if (line === undefined || !line.startsWith(">")) {
      break;
    }
    peeled.push(line.startsWith("> ") ? line.slice(2) : line.slice(1));
    index += 1;
  }
  return { inner: parseBlocks(peeled, quoteDepth + 1), next: index };
}

function paragraph(text: string): JSONContent {
  const content = parseInline(text);
  return content.length === 0
    ? { type: "paragraph" }
    : { type: "paragraph", content };
}

function codeBlock(language: string | null, text: string): JSONContent {
  const node: JSONContent = {
    type: "codeBlock",
    attrs: { language },
  };
  if (text.length > 0) {
    node.content = [{ type: "text", text }];
  }
  return node;
}

function parseList(
  lines: string[],
  start: number,
): { node: JSONContent; next: number } {
  const first = lines[start] ?? "";
  const isTask = TASK.test(first);
  const isOrdered = !isTask && ORDERED.test(first);
  const items: JSONContent[] = [];
  let index = start;

  while (index < lines.length) {
    const line = lines[index] ?? "";
    const task = TASK.exec(line);
    const bullet = BULLET.exec(line);
    const ordered = ORDERED.exec(line);
    if (isTask) {
      if (task === null) {
        break;
      }
      items.push({
        type: "taskItem",
        attrs: { checked: task[2].toLowerCase() === "x" },
        content: [paragraph(task[3] ?? "")],
      });
      index += 1;
      continue;
    }
    if (isOrdered) {
      if (ordered === null) {
        break;
      }
      items.push({
        type: "listItem",
        content: [paragraph(ordered[3] ?? "")],
      });
      index += 1;
      continue;
    }
    if (bullet === null || TASK.test(line)) {
      break;
    }
    items.push({
      type: "listItem",
      content: [paragraph(bullet[2] ?? "")],
    });
    index += 1;
  }

  return {
    node: {
      type: isTask ? "taskList" : isOrdered ? "orderedList" : "bulletList",
      content: items,
    },
    next: index,
  };
}

/**
 * Parses leftover source from the start of each remainder so a shared `***`
 * run can close bold and then open italic (`**a***b*`) without seeing the
 * already-consumed stars as part of the next opener.
 */
function parseInline(text: string): JSONContent[] {
  const nodes: JSONContent[] = [];
  let rest = text;

  while (rest.length > 0) {
    const inlineCode = takeInlineCode(rest);
    if (inlineCode !== null) {
      const file = tryParseComposerFileChip(inlineCode.inner);
      if (file !== null) {
        nodes.push(file);
      } else {
        pushText(nodes, inlineCode.inner, [{ type: "code" }]);
      }
      rest = rest.slice(inlineCode.end);
      continue;
    }

    const prompt = takePromptToken(rest, nodes);
    if (prompt !== null) {
      nodes.push(prompt.node);
      rest = rest.slice(prompt.end);
      continue;
    }

    // `/command` chips are not reconstructed from plain text — bare `/…` is too
    // ambiguous with paths (`/usr/bin`, `a / b`). Session switch parks TipTap
    // JSON so slash chips survive; restart keeps the `/name` characters.
    if (rest[0] === "$") {
      pushText(nodes, "$", []);
      rest = rest.slice(1);
      continue;
    }

    if (rest.startsWith("[")) {
      const last = nodes.at(-1);
      // `![alt](url)` is split so `[` is at the start of `rest`; keep it text.
      const afterImageBang =
        last?.type === "text" &&
        typeof last.text === "string" &&
        last.text.endsWith("!");
      const link = afterImageBang ? null : parseLink(rest, 0);
      if (link !== null) {
        // Dangerous schemes stay literal Markdown so they never enter a mark
        // or the agent payload. Click-time `safeComposerHref` is the last line.
        if (isDangerousComposerHref(link.href)) {
          pushText(nodes, rest.slice(0, link.end), []);
          rest = rest.slice(link.end);
          continue;
        }
        const attrs: Record<string, string | null> = { href: link.href };
        if (link.title !== undefined) {
          attrs.title = link.title;
        }
        let inner = parseInline(link.label);
        inner = withMark(inner, { type: "link", attrs });
        nodes.push(...inner);
        rest = rest.slice(link.end);
        continue;
      }
    }

    const wrapped = takeWrapped(rest, 0);
    if (wrapped !== null) {
      let inner = parseInline(wrapped.inner);
      for (const mark of wrapped.marks) {
        inner = withMark(inner, mark);
      }
      nodes.push(...inner);
      rest = rest.slice(wrapped.end);
      continue;
    }

    const next = nextSpecial(rest, 1);
    pushText(nodes, rest.slice(0, next), []);
    rest = rest.slice(next);
  }

  return nodes;
}

/**
 * Restores `$skill` chips from `documentPlainText`. Slash commands stay text on
 * this path (ambiguous with paths); park TipTap JSON for lossless `/` chips.
 * Tokens glued to a preceding word character (`cost$x`) stay text.
 */
function takePromptToken(
  text: string,
  preceding: JSONContent[],
): { node: JSONContent; end: number } | null {
  const match = /^\$([A-Za-z][\w-]*)/.exec(text);
  if (match === null) {
    return null;
  }
  const last = preceding.at(-1);
  if (
    last?.type === "text" &&
    typeof last.text === "string" &&
    last.text.length > 0 &&
    /[A-Za-z0-9]$/.test(last.text)
  ) {
    return null;
  }
  return {
    node: {
      type: "promptToken",
      attrs: {
        kind: "skill",
        name: match[1]!,
      },
    },
    end: match[0].length,
  };
}

/** Line-range suffix on a path chip: `:12` or `:12-34`. */
const FILE_CHIP_RANGE = /^(.*):(\d+)(?:-(\d+))?$/;

/**
 * Backtick payloads that look like workspace paths become file chips; plain
 * identifiers stay inline code so `` `code` `` does not become a chip.
 */
function tryParseComposerFileChip(inner: string): JSONContent | null {
  if (inner.length === 0 || inner.includes("`")) {
    return null;
  }
  let path = inner;
  let startLine: number | null = null;
  let endLine: number | null = null;
  const ranged = FILE_CHIP_RANGE.exec(inner);
  if (ranged !== null) {
    path = ranged[1]!;
    startLine = Number(ranged[2]);
    endLine = ranged[3] === undefined ? startLine : Number(ranged[3]);
  }
  if (!looksLikeComposerFilePath(path)) {
    return null;
  }
  return {
    type: "composerFile",
    attrs: {
      path,
      startLine,
      endLine,
      kind: "file",
    },
  };
}

/** Common source/doc extensions; bare `v1.0` / globs stay inline code. */
const SOURCE_FILE_EXT =
  /\.(?:ts|tsx|js|jsx|mjs|cjs|json|md|mdx|css|scss|html|rs|py|go|java|kt|swift|c|cc|cpp|h|hpp|cs|rb|php|sh|bash|zsh|yaml|yml|toml|xml|svg|png|jpg|jpeg|gif|webp|txt|sql|proto|graphql|vue|svelte|dart|lua|r|zig|ex|exs)$/i;

/** Path-like enough to prefer a chip over inline code on text-only restore. */
function looksLikeComposerFilePath(path: string): boolean {
  if (path.length === 0 || /\s/.test(path) || /[*?]/.test(path)) {
    return false;
  }
  if (path.includes("/") || path.includes("\\")) {
    return true;
  }
  if (path.startsWith(".") && path.length > 1) {
    return true;
  }
  // Semver-looking tokens (`v1.0`, `1.2.3`) are intentional inline code.
  if (/^v?\d+(?:\.\d+)+$/i.test(path)) {
    return false;
  }
  return SOURCE_FILE_EXT.test(path);
}

/**
 * GFM inline link, including an optional quoted title. Images (`![...](...)`)
 * stay text; the prompt box does not own image nodes.
 */
function parseLink(
  text: string,
  index: number,
): { label: string; href: string; title?: string; end: number } | null {
  if (index > 0 && text[index - 1] === "!") {
    return null;
  }
  const closeLabel = text.indexOf("](", index);
  if (closeLabel === -1 || closeLabel === index + 1) {
    return null;
  }
  const label = text.slice(index + 1, closeLabel);
  if (label.includes("[")) {
    return null;
  }
  const taken = takeLinkDestination(text, closeLabel + 2);
  if (taken === null) {
    return null;
  }
  let cursor = taken.end;
  while (cursor < text.length && /\s/.test(text[cursor] ?? "")) {
    cursor += 1;
  }
  let title: string | undefined;
  const quote = text[cursor];
  if (quote === '"' || quote === "'") {
    const takenTitle = takeQuotedLinkTitle(text, cursor, quote);
    if (takenTitle === null) {
      return null;
    }
    title = takenTitle.title.length === 0 ? undefined : takenTitle.title;
    cursor = takenTitle.end + 1;
    while (cursor < text.length && /\s/.test(text[cursor] ?? "")) {
      cursor += 1;
    }
  }
  if (text[cursor] !== ")") {
    return null;
  }
  return { label, href: taken.href, title, end: cursor + 1 };
}

/**
 * CommonMark inline code: a run of opening backticks closed by the same
 * number, not as part of a longer run. One leading/trailing space is padding
 * so a payload that starts with a backtick can still round-trip.
 */
function takeInlineCode(text: string): { inner: string; end: number } | null {
  if (text[0] !== "`") {
    return null;
  }
  let ticks = 0;
  while (text[ticks] === "`") {
    ticks += 1;
  }
  let index = ticks;
  while (index < text.length) {
    if (text[index] !== "`") {
      index += 1;
      continue;
    }
    let run = 0;
    while (text[index + run] === "`") {
      run += 1;
    }
    if (run === ticks) {
      let inner = text.slice(ticks, index);
      if (inner.length >= 2 && inner.startsWith(" ") && inner.endsWith(" ")) {
        inner = inner.slice(1, -1);
      }
      return { inner, end: index + ticks };
    }
    index += run;
  }
  return null;
}

/**
 * Reads a quoted link title, honoring `\\` and `\"` / `\'` so serialization's
 * escape of quotes does not close the title early.
 */
function takeQuotedLinkTitle(
  text: string,
  start: number,
  quote: string,
): { title: string; end: number } | null {
  let index = start + 1;
  let title = "";
  while (index < text.length) {
    const char = text[index];
    if (char === "\\" && index + 1 < text.length) {
      title += text[index + 1];
      index += 2;
      continue;
    }
    if (char === quote) {
      return { title, end: index };
    }
    title += char;
    index += 1;
  }
  return null;
}

function takeLinkDestination(
  text: string,
  start: number,
): { href: string; end: number } | null {
  if (text[start] === "<") {
    const close = text.indexOf(">", start + 1);
    if (close === -1) {
      return null;
    }
    const href = text.slice(start + 1, close);
    return href.length === 0 ? null : { href, end: close + 1 };
  }
  let cursor = start;
  let depth = 0;
  while (cursor < text.length) {
    const char = text[cursor];
    if (
      char === undefined ||
      (depth === 0 && (char === ")" || /\s/.test(char)))
    ) {
      break;
    }
    if (char === "(") {
      depth += 1;
    } else if (char === ")") {
      depth -= 1;
    }
    cursor += 1;
  }
  const href = text.slice(start, cursor);
  return href.length === 0 ? null : { href, end: cursor };
}

/**
 * Longest delimiter first, then the nearest closer. Extra stars after a
 * closer stay on the remainder (`**bold***em*`) which parseInline slices off.
 */
function takeWrapped(
  text: string,
  index: number,
): { inner: string; marks: MarkSpec[]; end: number } | null {
  const delimiters: Array<{ token: string; marks: MarkSpec[] }> = [
    { token: "***", marks: [{ type: "italic" }, { type: "bold" }] },
    { token: "**", marks: [{ type: "bold" }] },
    { token: "__", marks: [{ type: "bold" }] },
    { token: "~~", marks: [{ type: "strike" }] },
    { token: "==", marks: [{ type: "highlight" }] },
    { token: "*", marks: [{ type: "italic" }] },
    { token: "_", marks: [{ type: "italic" }] },
  ];
  for (const delimiter of delimiters) {
    if (!gfmCanOpen(text, index, delimiter.token)) {
      continue;
    }
    const start = index + delimiter.token.length;
    const close = text.indexOf(delimiter.token, start);
    if (close > start && gfmCanClose(text, close, delimiter.token)) {
      return {
        inner: text.slice(start, close),
        marks: delimiter.marks,
        end: close + delimiter.token.length,
      };
    }
  }
  return null;
}

/**
 * Same flanking as ComposerBold / ComposerItalic / ComposerStrike: no space
 * after the opener or before the closer, and `_` does not fire inside snake_case.
 */
function gfmCanOpen(text: string, index: number, token: string): boolean {
  if (!text.startsWith(token, index)) {
    return false;
  }
  const next = text[index + token.length];
  if (next !== undefined && /\s/.test(next)) {
    return false;
  }
  if (token === "***" || token === "**" || token === "*") {
    if (index > 0 && text[index - 1] === "*") {
      return false;
    }
    if (next === "*") {
      return false;
    }
  }
  if (token === "_" || token === "__") {
    if (index > 0 && text[index - 1] === "_") {
      return false;
    }
    if (
      token === "_" &&
      index > 0 &&
      /[A-Za-z0-9]/.test(text[index - 1] ?? "")
    ) {
      return false;
    }
    if (token === "_" && next === "_") {
      return false;
    }
  }
  return true;
}

function gfmCanClose(text: string, close: number, token: string): boolean {
  const prev = text[close - 1];
  if (prev !== undefined && /\s/.test(prev)) {
    return false;
  }
  // Extra `*` / `_` after the closer stay in the run so `**bold***em*` can
  // close bold with two stars and open italic with the leftover one. Openers
  // already refuse to start in the middle of a longer token.
  const after = text[close + token.length];
  if (token === "_" && after !== undefined && /[A-Za-z0-9]/.test(after)) {
    return false;
  }
  return true;
}

function nextSpecial(text: string, from: number): number {
  for (let index = from; index < text.length; index += 1) {
    const char = text[index];
    if (
      char === "`" ||
      char === "[" ||
      char === "*" ||
      char === "_" ||
      char === "~" ||
      char === "=" ||
      char === "$"
    ) {
      return index;
    }
  }
  return text.length;
}

function withMark(nodes: JSONContent[], mark: MarkSpec): JSONContent[] {
  return nodes.map((node) => {
    if (node.type !== "text" || typeof node.text !== "string") {
      return node;
    }
    return {
      ...node,
      marks: [...(node.marks ?? []), mark],
    };
  });
}

function pushText(nodes: JSONContent[], text: string, marks: MarkSpec[]): void {
  if (text.length === 0) {
    return;
  }
  const node: JSONContent = { type: "text", text };
  if (marks.length > 0) {
    node.marks = marks;
  }
  nodes.push(node);
}
