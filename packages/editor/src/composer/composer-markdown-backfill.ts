import { Extension } from "@tiptap/core";
import { Fragment, type Node } from "@tiptap/pm/model";
import {
  Plugin,
  PluginKey,
  type EditorState,
  type Transaction,
} from "@tiptap/pm/state";
import {
  looksLikeComposerMarkdown,
  markdownToComposerContent,
} from "./composer-markdown.ts";

const BACKFILL_META = "composerMarkdownBackfill";

/**
 * Input rules only see text before the caret, so wrapping `a==` with `==` (or
 * `**` / `~~` / `` ` ``) stays source until the user confirms. Convert leftover
 * Markdown only after a trailing space or a newline — converting on the first
 * `*` of `*bold**` would steal italic and break the `**` wrap.
 */
export const ComposerMarkdownBackfill = Extension.create({
  name: "composerMarkdownBackfill",

  addProseMirrorPlugins() {
    return [
      new Plugin({
        key: new PluginKey("composerMarkdownBackfill"),
        appendTransaction(transactions, oldState, newState) {
          if (
            !transactions.some((transaction) => transaction.docChanged) ||
            transactions.some(
              (transaction) =>
                transaction.getMeta(BACKFILL_META) ||
                transaction.getMeta("composerMarkdownRevert") === true,
            )
          ) {
            return null;
          }
          const targets = confirmTargetPositions(
            oldState,
            newState,
            transactions,
          );
          if (targets.length === 0) {
            return null;
          }
          return backfillPlainMarkdownBlocks(newState, targets);
        },
      }),
    ];
  },
});

/**
 * Only the line the user just confirmed: the selected textblock when it gains
 * trailing whitespace, or the block that kept content after a mid/end split.
 * Skips full-doc scans so unrelated typing cannot wake a stuck leftover line.
 */
function confirmTargetPositions(
  oldState: EditorState,
  newState: EditorState,
  transactions: readonly Transaction[],
): number[] {
  const targets: number[] = [];
  const seen = new Set<number>();

  const add = (pos: number, node: Node): void => {
    if (
      seen.has(pos) ||
      !isPlainUnmarkedTextblock(node) ||
      !looksLikeComposerMarkdown(node.textContent)
    ) {
      return;
    }
    seen.add(pos);
    targets.push(pos);
  };

  const new$ = newState.selection.$from;
  if (
    new$.parent.isTextblock &&
    new$.parent.type.name !== "codeBlock" &&
    /\s$/.test(new$.parent.textContent)
  ) {
    add(new$.before(new$.depth), new$.parent);
  }

  if (textblockCount(newState.doc) > textblockCount(oldState.doc)) {
    const old$ = oldState.selection.$from;
    // Caret at the start inserts an empty block above; that is not a confirm.
    if (old$.parent.isTextblock && old$.parentOffset > 0) {
      let mapped = old$.before(old$.depth);
      for (const transaction of transactions) {
        mapped = transaction.mapping.map(mapped);
      }
      const node = newState.doc.nodeAt(mapped);
      if (node !== null && node.isTextblock) {
        add(mapped, node);
      }
    }
  }

  return targets;
}

/** Counts paragraphs, headings, and other leaves so a split can confirm leftover source. */
function textblockCount(doc: Node): number {
  let count = 0;
  doc.descendants((node) => {
    if (node.isTextblock) {
      count += 1;
      return false;
    }
    return true;
  });
  return count;
}

/**
 * Replaces only the confirmed leftover blocks when the parent schema allows it
 * (a list item still cannot become a heading).
 */
function backfillPlainMarkdownBlocks(
  state: EditorState,
  targetPositions: readonly number[],
): Transaction | null {
  const replacements: { from: number; to: number; nodes: Node[] }[] = [];

  for (const pos of targetPositions) {
    const node = state.doc.nodeAt(pos);
    if (node === null || !isPlainUnmarkedTextblock(node)) {
      continue;
    }
    const text = node.textContent;
    if (!looksLikeComposerMarkdown(text)) {
      continue;
    }
    const jsonBlocks = markdownToComposerContent(text).content ?? [];
    if (!shouldReplacePlainBlock(node, jsonBlocks)) {
      continue;
    }
    let nodes: Node[];
    try {
      nodes = jsonBlocks.map((block) => state.schema.nodeFromJSON(block));
    } catch {
      continue;
    }
    const $pos = state.doc.resolve(pos);
    if (
      !$pos.parent.canReplace(
        $pos.index(),
        $pos.index() + 1,
        Fragment.from(nodes),
      )
    ) {
      continue;
    }
    replacements.push({ from: pos, to: pos + node.nodeSize, nodes });
  }

  if (replacements.length === 0) {
    return null;
  }

  replacements.sort((left, right) => right.from - left.from);
  const { tr } = state;
  for (const replacement of replacements) {
    replacePlainBlock(tr, replacement.from, replacement.to, replacement.nodes);
  }
  tr.setMeta(BACKFILL_META, true);
  return tr;
}

/**
 * Same-type replacements keep the outer textblock so a trailing space or
 * split-block mapping stays on the converted line instead of jumping.
 */
function replacePlainBlock(
  tr: Transaction,
  from: number,
  to: number,
  nodes: Node[],
): void {
  const current = tr.doc.nodeAt(from);
  if (
    current !== null &&
    nodes.length === 1 &&
    nodes[0] !== undefined &&
    nodes[0].type === current.type
  ) {
    tr.replaceWith(from + 1, to - 1, nodes[0].content);
    return;
  }
  tr.replaceWith(from, to, nodes);
}

/** True when the block is still source text, not already-converted marks or chips. */
function isPlainUnmarkedTextblock(node: Node): boolean {
  if (!node.isTextblock || node.type.name === "codeBlock") {
    return false;
  }
  for (let index = 0; index < node.childCount; index += 1) {
    const child = node.child(index);
    if (!child.isText || child.marks.length > 0) {
      return false;
    }
  }
  return true;
}

/**
 * Skip no-op replacements (image syntax stays text) and fence/rule openers so
 * typing `***both***` or ```C++ is not stolen after the first delimiter.
 */
function shouldReplacePlainBlock(
  node: Node,
  jsonBlocks: ReturnType<typeof markdownToComposerContent>["content"],
): boolean {
  if (jsonBlocks === undefined || jsonBlocks.length === 0) {
    return false;
  }
  if (
    jsonBlocks.some(
      (block) =>
        block?.type === "codeBlock" || block?.type === "horizontalRule",
    )
  ) {
    return false;
  }
  if (jsonBlocks.length !== 1) {
    return true;
  }
  const parsed = jsonBlocks[0];
  if (parsed === undefined || parsed.type !== node.type.name) {
    return true;
  }
  return (
    JSON.stringify(parsed.content ?? []) !==
    JSON.stringify(node.content.toJSON())
  );
}
