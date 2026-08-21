const FENCE_SPLIT = /(```[\s\S]*?```)/;

/**
 * TipTap `documentPlainText` joins blocks with a single newline. GFM would
 * treat that as a soft break, so expand to paragraph breaks outside fences
 * before read-only Markdown render.
 */
export function prepareUserMessageMarkdown(content: string): string {
  const normalized = content.replace(/\r\n/g, "\n");
  return normalized
    .split(FENCE_SPLIT)
    .map((part) => {
      if (part.startsWith("```")) {
        return part;
      }
      return part.replace(/([^\n])\n(?=[^\n])/g, "$1\n\n");
    })
    .join("");
}

type MdastNode = {
  type: string;
  value?: string;
  children?: MdastNode[];
  data?: { hName?: string; hProperties?: Record<string, unknown> };
};

const HIGHLIGHT_PATTERN = /==(?!\s)([^=]+?)(?<!\s)==/g;

/**
 * Turns Typora-style `==highlight==` (composer surface, not GFM) into `<mark>`
 * via hName, skipping code and inlineCode values.
 */
export function remarkComposerHighlight() {
  return (tree: MdastNode) => {
    transformPhrasing(tree);
  };
}

function transformPhrasing(node: MdastNode): void {
  if (node.type === "code" || node.type === "inlineCode") {
    return;
  }
  const { children } = node;
  if (children === undefined) {
    return;
  }
  const next: MdastNode[] = [];
  for (const child of children) {
    if (child.type === "text" && typeof child.value === "string") {
      next.push(...splitHighlightText(child.value));
      continue;
    }
    transformPhrasing(child);
    next.push(child);
  }
  node.children = next;
}

function splitHighlightText(value: string): MdastNode[] {
  HIGHLIGHT_PATTERN.lastIndex = 0;
  if (!HIGHLIGHT_PATTERN.test(value)) {
    return [{ type: "text", value }];
  }
  HIGHLIGHT_PATTERN.lastIndex = 0;
  const nodes: MdastNode[] = [];
  let lastIndex = 0;
  for (const match of value.matchAll(HIGHLIGHT_PATTERN)) {
    const index = match.index ?? 0;
    const inner = match[1];
    if (inner === undefined) {
      continue;
    }
    if (index > lastIndex) {
      nodes.push({ type: "text", value: value.slice(lastIndex, index) });
    }
    nodes.push({
      type: "emphasis",
      data: {
        hName: "mark",
        hProperties: { className: "composer-user-highlight" },
      },
      children: [{ type: "text", value: inner }],
    });
    lastIndex = index + match[0].length;
  }
  if (lastIndex < value.length) {
    nodes.push({ type: "text", value: value.slice(lastIndex) });
  }
  return nodes.length > 0 ? nodes : [{ type: "text", value }];
}
