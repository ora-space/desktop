import type { Components } from "react-markdown";
import type { PromptTokenKind } from "@ora/editor/composer";
import { PromptTokenChip } from "./prompt-token-chip";

/** Element name the remark pass emits; only this module maps it to a chip. */
const PROMPT_TOKEN_TAG = "ora-prompt-token";

interface MdastNode {
  type: string;
  value?: string;
  children?: MdastNode[];
  data?: { hName?: string; hProperties?: Record<string, unknown> };
}

/**
 * `$skill`, `/command`, and `@role` tokens as the composer emits them in the
 * sent prompt's plain text. A leading alphanumeric boundary is rejected so a
 * glue case like `cost$x` or an email handle stays plain text; a slash is also
 * rejected when it is bookended by `/` so a multi-segment path like `/usr/bin`
 * is not misread as a pair of command chips.
 */
const TOKEN_PATTERN =
  /(?<![A-Za-z0-9/])([$/@])([A-Za-z][\w-]*)(?![A-Za-z0-9/])/g;

/**
 * Rebuilds the skill, command, and role chips the composer showed while the
 * prompt was being written.
 *
 * Sending flattens those tokens to Markdown (`$skill`, `/command`, `@role`)
 * because that is what the agent reads, but the same text is what history
 * replays — so without this the user's own tokens come back as a wall of raw
 * characters. File-quote chips are handled by their own pass; only plain-text
 * token fragments are split here, never code or inline code.
 */
export function remarkComposerPromptTokens() {
  return (tree: MdastNode) => {
    splitPromptTokens(tree);
  };
}

function splitPromptTokens(node: MdastNode): void {
  if (node.type === "code" || node.type === "inlineCode") return;
  const { children } = node;
  if (children === undefined) return;
  const next: MdastNode[] = [];
  for (const child of children) {
    // Skip nodes already carrying a custom element (file-quote/highlight chips
    // are `text` with `data.hName`); splitting them on `$`/`/`/`@` would strip
    // the hName and turn a chip back into raw text.
    if (
      child.type === "text" &&
      typeof child.value === "string" &&
      child.data?.hName === undefined
    ) {
      next.push(...splitTokenText(child.value));
      continue;
    }
    splitPromptTokens(child);
    next.push(child);
  }
  node.children = next;
}

function splitTokenText(value: string): MdastNode[] {
  TOKEN_PATTERN.lastIndex = 0;
  const nodes: MdastNode[] = [];
  let lastIndex = 0;
  for (const match of value.matchAll(TOKEN_PATTERN)) {
    const index = match.index ?? 0;
    if (index > lastIndex) {
      nodes.push({ type: "text", value: value.slice(lastIndex, index) });
    }
    nodes.push(chipNode(tokenKind(match[1]), match[2] ?? ""));
    lastIndex = index + match[0].length;
  }
  if (lastIndex < value.length) {
    nodes.push({ type: "text", value: value.slice(lastIndex) });
  }
  return nodes.length > 0 ? nodes : [{ type: "text", value }];
}

function tokenKind(prefix: string | undefined): PromptTokenKind {
  if (prefix === "/") return "command";
  if (prefix === "@") return "role";
  return "skill";
}

function chipNode(kind: PromptTokenKind, name: string): MdastNode {
  return {
    type: "text",
    value: "",
    data: { hName: PROMPT_TOKEN_TAG, hProperties: { kind, name } },
  };
}

/**
 * Maps the chip element onto React. The cast is unavoidable: react-markdown
 * types `components` against known HTML tag names, and a custom name is the
 * only way to carry parsed attrs to the renderer without re-parsing the token.
 */
export const promptTokenMarkdownComponents = {
  [PROMPT_TOKEN_TAG]: (props: Record<string, unknown>) => (
    <PromptTokenChip
      kind={String(props.kind) as PromptTokenKind}
      name={String(props.name)}
    />
  ),
} as unknown as Components;
