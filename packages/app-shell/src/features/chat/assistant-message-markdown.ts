const FENCE_SPLIT = /(```[\s\S]*?```)/;

interface MdastNode {
  type: string;
  value?: string;
  children?: MdastNode[];
}

/** Keeps blank lines beyond Markdown's normal paragraph separator visible. */
export function prepareAssistantMessageMarkdown(content: string): string {
  return content
    .replace(/\r\n?/g, "\n")
    .split(FENCE_SPLIT)
    .map((part) => {
      if (part.startsWith("```")) {
        return part;
      }
      return part.replace(
        /([^\n])(\n{3,})(?=[^\n])/g,
        (_match, preceding: string, lineBreaks: string) =>
          `${preceding}\n\n${"\u00a0\n\n".repeat(lineBreaks.length - 2)}`,
      );
    })
    .join("");
}

/** Preserves ordinary response newlines instead of letting GFM render them as spaces. */
export function remarkSoftBreaks() {
  return (tree: MdastNode) => {
    replaceSoftBreaks(tree);
  };
}

/** Replaces newlines in prose text with mdast break nodes while leaving code untouched. */
function replaceSoftBreaks(node: MdastNode): void {
  if (node.type === "code" || node.type === "inlineCode") {
    return;
  }
  if (node.children === undefined) {
    return;
  }

  const children: MdastNode[] = [];
  for (const child of node.children) {
    if (
      child.type === "text" &&
      typeof child.value === "string" &&
      child.value.includes("\n")
    ) {
      const lines = child.value.split("\n");
      lines.forEach((line, index) => {
        if (index > 0) {
          children.push({ type: "break" });
        }
        if (line.length > 0) {
          children.push({ type: "text", value: line });
        }
      });
      continue;
    }
    replaceSoftBreaks(child);
    children.push(child);
  }
  node.children = children;
}
