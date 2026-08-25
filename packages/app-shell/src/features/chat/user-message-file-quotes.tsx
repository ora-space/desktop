import {
  composerFileAttrsFromUnknown,
  parseComposerFileQuote,
  type ComposerFileAttrs,
} from "@ora/editor/composer";
import type { Components } from "react-markdown";
import { FileRefChip } from "../file-ref-chip";

/** Element name the remark pass emits; only this module maps it to a chip. */
const FILE_QUOTE_TAG = "ora-file-quote";

interface MdastNode {
  type: string;
  lang?: string | null;
  meta?: string | null;
  value?: string;
  children?: MdastNode[];
  data?: { hName?: string; hProperties?: Record<string, unknown> };
}

/**
 * Renders the fenced quote payloads inside a sent prompt as the chips the
 * composer showed while it was being written.
 *
 * Sending flattens chips to Markdown (`composerFilePlainText`) because that is
 * what the agent reads, but the same text is what history replays — so without
 * this the user's own quote comes back as a wall of source. Quotes that were
 * adjacent in the composer serialize as back-to-back fences and are folded into
 * one paragraph again, keeping them on a single line.
 */
export function remarkComposerFileQuote() {
  return (tree: MdastNode) => {
    const next: MdastNode[] = [];
    let openChipLine: MdastNode | null = null;
    for (const child of tree.children ?? []) {
      const attrs = fileQuoteAttrs(child);
      if (attrs === null) {
        openChipLine = null;
        next.push(child);
        continue;
      }
      if (openChipLine !== null) {
        openChipLine.children?.push(chipNode(attrs));
        continue;
      }
      openChipLine = { type: "paragraph", children: [chipNode(attrs)] };
      next.push(openChipLine);
    }
    tree.children = next;
  };
}

/**
 * Maps the chip element onto React. The cast is unavoidable: react-markdown
 * types `components` against known HTML tag names, and a custom name is the
 * only way to carry parsed attrs to the renderer without re-parsing the fence.
 */
export const fileQuoteMarkdownComponents = {
  [FILE_QUOTE_TAG]: (props: Record<string, unknown>) => (
    <FileRefChip attrs={composerFileAttrsFromUnknown(props)} />
  ),
} as unknown as Components;

/** Chip attrs for a fenced quote block, or null for any other node. */
function fileQuoteAttrs(node: MdastNode): ComposerFileAttrs | null {
  if (node.type !== "code") return null;
  // A path with a space lands in `meta`; the info string is both halves.
  const info = [node.lang ?? "", node.meta ?? ""].filter(Boolean).join(" ");
  return parseComposerFileQuote(info, node.value ?? "");
}

function chipNode(attrs: ComposerFileAttrs): MdastNode {
  // hast walks every property it is given, so absent quote metadata is dropped
  // rather than handed over as `undefined`.
  const hProperties = Object.fromEntries(
    Object.entries(attrs).filter(([, value]) => value !== undefined),
  );
  return {
    type: "text",
    value: "",
    data: { hName: FILE_QUOTE_TAG, hProperties },
  };
}
