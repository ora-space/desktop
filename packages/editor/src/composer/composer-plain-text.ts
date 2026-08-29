import type { JSONContent } from "@tiptap/core";
import type { Mark, Node as PmNode } from "@tiptap/pm/model";
import {
  composerFileAttrsFromNode,
  composerFilePlainText,
} from "./composer-file";

function leafPlainText(node: PmNode): string {
  switch (node.type.name) {
    case "hardBreak":
      return "\n";
    case "promptToken": {
      const kind = node.attrs.kind;
      const prefix = kind === "command" ? "/" : kind === "role" ? "@" : "$";
      return `${prefix}${String(node.attrs.name)}`;
    }
    case "composerFile":
      return composerFilePlainText(composerFileAttrsFromNode(node));
    case "horizontalRule":
      return "---";
    default:
      return "";
  }
}

/**
 * CommonMark inline code: the fence is one backtick longer than the longest
 * run inside the payload, with space padding so an inner `` ` `` cannot close
 * the span. A single backtick stays the everyday `` `code` `` form.
 */
function wrapInlineCode(text: string): string {
  const longestRun =
    text.match(/`+/g)?.reduce((max, run) => Math.max(max, run.length), 0) ?? 0;
  const fence = "`".repeat(Math.max(1, longestRun + 1));
  if (
    longestRun > 0 ||
    text.startsWith(" ") ||
    text.endsWith(" ") ||
    text.startsWith("`") ||
    text.endsWith("`")
  ) {
    return `${fence} ${text} ${fence}`;
  }
  return `${fence}${text}${fence}`;
}

/**
 * Escapes a Markdown link title so parse can find the real closing quote.
 * Backslashes first, then quotes — otherwise `\"` would split into a stray `"`.
 */
function escapeLinkTitle(title: string): string {
  return title.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

function wrapInlineMarkdown(text: string, marks: readonly Mark[]): string {
  let out = text;
  const names = new Set(marks.map((mark) => mark.type.name));
  if (names.has("code")) {
    out = wrapInlineCode(out);
  } else {
    if (names.has("strike")) {
      out = `~~${out}~~`;
    }
    if (names.has("italic")) {
      out = `*${out}*`;
    }
    if (names.has("bold")) {
      out = `**${out}**`;
    }
    if (names.has("highlight")) {
      out = `==${out}==`;
    }
  }
  const link = marks.find((mark) => mark.type.name === "link");
  if (link === undefined) {
    return out;
  }
  const href = String(link.attrs.href ?? "");
  if (href.length === 0 || href === text) {
    return out;
  }
  const title = link.attrs.title;
  if (typeof title === "string" && title.length > 0) {
    return `[${out}](${href} "${escapeLinkTitle(title)}")`;
  }
  return `[${out}](${href})`;
}

/**
 * One marked text run as Markdown source. Used when Backspace restores
 * delimiters without rewriting the rest of the line.
 */
export function inlineMarksPlainText(
  text: string,
  marks: readonly Mark[],
): string {
  return wrapInlineMarkdown(text, marks);
}

function serializeText(node: PmNode): string {
  return wrapInlineMarkdown(node.text ?? "", node.marks);
}

function isComposerChipLeaf(node: PmNode): boolean {
  return node.type.name === "composerFile" || node.type.name === "promptToken";
}

function serializeInline(node: PmNode): string {
  let out = "";
  let prevWasChip = false;
  node.forEach((child) => {
    if (child.isText) {
      out += serializeText(child);
      prevWasChip = false;
      return;
    }
    if (child.isLeaf) {
      // Chips sit adjacent in the doc (no selectable spaces between them);
      // still emit a space in the agent payload so paths stay separated.
      if (prevWasChip && isComposerChipLeaf(child)) {
        out += " ";
      }
      out += leafPlainText(child);
      prevWasChip = isComposerChipLeaf(child);
      return;
    }
    out += serializeInline(child);
    prevWasChip = false;
  });
  return out;
}

function joinChildBlocks(node: PmNode, indent: string): string {
  const parts: string[] = [];
  node.forEach((child) => {
    parts.push(serializeBlock(child, indent));
  });
  return parts.join("\n");
}

function serializeListItem(
  node: PmNode,
  indent: string,
  marker: string,
): string {
  const parts: string[] = [];
  let first = true;
  node.forEach((child) => {
    if (first && child.type.name === "paragraph") {
      parts.push(`${indent}${marker}${serializeInline(child)}`);
      first = false;
      return;
    }
    first = false;
    parts.push(serializeBlock(child, `${indent}  `));
  });
  return parts.join("\n");
}

function serializeList(
  node: PmNode,
  indent: string,
  markerFor: (index: number, child: PmNode) => string,
): string {
  const parts: string[] = [];
  let index = 0;
  node.forEach((child) => {
    parts.push(serializeListItem(child, indent, markerFor(index, child)));
    index += 1;
  });
  return parts.join("\n");
}

function serializeBlock(node: PmNode, indent = ""): string {
  switch (node.type.name) {
    case "horizontalRule":
      return `${indent}---`;
    case "heading": {
      const level = Math.min(Math.max(Number(node.attrs.level ?? 1), 1), 6);
      const inline = serializeInline(node);
      return inline.length === 0
        ? `${indent}${"#".repeat(level)}`
        : `${indent}${"#".repeat(level)} ${inline}`;
    }
    case "codeBlock": {
      const language =
        node.attrs.language === null ||
        node.attrs.language === undefined ||
        node.attrs.language === ""
          ? ""
          : String(node.attrs.language);
      const text = node.textContent;
      // Longer than any backtick run in the body so a nested ``` line cannot
      // close the fence on parse (CommonMark closing-fence rule).
      const longestRun =
        text.match(/`+/g)?.reduce((max, run) => Math.max(max, run.length), 0) ??
        0;
      const fence = "`".repeat(Math.max(3, longestRun + 1));
      return `${indent}${fence}${language}\n${text}\n${indent}${fence}`;
    }
    case "blockquote": {
      const inner = joinChildBlocks(node, "");
      return inner
        .split("\n")
        .map((line) =>
          line.length === 0 ? `${indent}>` : `${indent}> ${line}`,
        )
        .join("\n");
    }
    case "bulletList":
      return serializeList(node, indent, () => "- ");
    case "orderedList": {
      const start = Number(node.attrs.start ?? 1);
      return serializeList(node, indent, (index) => `${start + index}. `);
    }
    case "taskList":
      return serializeList(node, indent, (_index, child) =>
        child.attrs.checked === true ? "- [x] " : "- [ ] ",
      );
    case "listItem":
    case "taskItem":
      return serializeListItem(node, indent, "- ");
    case "paragraph":
      return `${indent}${serializeInline(node)}`;
    default:
      if (node.isLeaf) {
        return `${indent}${leafPlainText(node)}`;
      }
      return joinChildBlocks(node, indent);
  }
}

/**
 * One textblock as Markdown source (marks, heading prefixes, chips). Used to
 * put the caret back into delimiter-editing after a live conversion.
 */
export function textblockPlainText(node: PmNode): string {
  return serializeBlock(node);
}

/**
 * Reads composer documents as textarea-like plain text: hard breaks and
 * block boundaries both become a single newline so ACP/HITL payloads stay strings.
 * Structured blocks keep Markdown prefixes so the agent sees headings, lists,
 * fences, and marks.
 */
export function documentPlainText(doc: PmNode): string {
  const parts: string[] = [];
  doc.forEach((node) => {
    parts.push(serializeBlock(node));
  });
  return parts.join("\n");
}

/**
 * Builds Tiptap JSON from raw lines with no Markdown parse, so `<` stays text.
 * Composer seed/replace uses `markdownToComposerContent` instead.
 */
export function plainTextToComposerContent(text: string): JSONContent {
  if (text.length === 0) {
    return { type: "doc", content: [{ type: "paragraph" }] };
  }

  return {
    type: "doc",
    content: text.split("\n").map((line) =>
      line.length === 0
        ? { type: "paragraph" }
        : {
            type: "paragraph",
            content: [{ type: "text", text: line }],
          },
    ),
  };
}
