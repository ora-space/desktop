import { liftEmptyBlock } from "@tiptap/pm/commands";
import { liftListItem } from "@tiptap/pm/schema-list";
import { TextSelection } from "@tiptap/pm/state";
import type { NodeType } from "@tiptap/pm/model";
import type { EditorView } from "@tiptap/pm/view";
import {
  convertMarkdownFenceOpener,
  handleComposerCodeEnter,
} from "./composer-code-fence";

export type ComposerEnterAction = "handled" | "submit";

const LIST_WRAPPERS = new Set(["bulletList", "orderedList", "taskList"]);
const LIST_ITEMS = new Set(["listItem", "taskItem"]);

/**
 * Enter leaves the current structure the way it leaves a code fence: one key
 * returns to body text. Shift+Enter is the newline inside the structure.
 */
export function resolveComposerEnter(view: EditorView): ComposerEnterAction {
  if (handleComposerCodeEnter(view) || convertMarkdownFenceOpener(view)) {
    return "handled";
  }
  if (exitComposerStructure(view)) {
    return "handled";
  }
  return "submit";
}

/**
 * Exits a quote, list, or heading. Empty wrappers collapse in place; otherwise
 * a body paragraph is inserted after the structure.
 */
export function exitComposerStructure(view: EditorView): boolean {
  const { $from } = view.state.selection;
  for (let depth = $from.depth; depth > 0; depth -= 1) {
    const name = $from.node(depth).type.name;
    if (name === "heading") {
      return exitHeading(view, depth);
    }
    if (LIST_ITEMS.has(name)) {
      return exitList(view, depth);
    }
    if (name === "blockquote") {
      return exitBlockquote(view, depth);
    }
  }
  return false;
}

function exitHeading(view: EditorView, depth: number): boolean {
  const { state } = view;
  const { $from } = state.selection;
  const paragraph = state.schema.nodes.paragraph;
  if (paragraph === undefined) {
    return false;
  }
  if ($from.parent.content.size === 0) {
    const from = $from.before(depth);
    view.dispatch(
      state.tr
        .setBlockType(from, from + $from.node(depth).nodeSize, paragraph)
        .scrollIntoView(),
    );
    return true;
  }
  if ($from.parentOffset === $from.parent.content.size) {
    return insertParagraphAfter(view, depth);
  }
  const tr = state.tr;
  try {
    tr.split($from.pos);
    const mapped = tr.mapping.map($from.pos);
    const $mapped = tr.doc.resolve(mapped);
    const start = $mapped.before($mapped.depth);
    const end = start + $mapped.parent.nodeSize;
    tr.setBlockType(start, end, paragraph);
    tr.setSelection(TextSelection.near(tr.doc.resolve(start + 1)));
  } catch {
    // A failed split must not fall through to submit; insert a body paragraph
    // after the heading so Enter still leaves the structure.
    return insertParagraphAfter(view, depth);
  }
  view.dispatch(tr.scrollIntoView());
  return true;
}

function exitList(view: EditorView, itemDepth: number): boolean {
  const { state } = view;
  const { $from } = state.selection;
  const item = $from.node(itemDepth);
  if (item.content.size === 0) {
    return liftCurrentListItem(view, item.type);
  }
  for (let depth = itemDepth - 1; depth > 0; depth -= 1) {
    if (LIST_WRAPPERS.has($from.node(depth).type.name)) {
      return insertParagraphAfter(view, depth);
    }
  }
  return false;
}

function exitBlockquote(view: EditorView, depth: number): boolean {
  const { state } = view;
  const { $from } = state.selection;
  if ($from.parent.content.size === 0) {
    return liftEmptyBlock(state, (tr) => view.dispatch(tr.scrollIntoView()));
  }
  return insertParagraphAfter(view, depth);
}

function liftCurrentListItem(view: EditorView, itemType: NodeType): boolean {
  return liftListItem(itemType)(view.state, (tr) =>
    view.dispatch(tr.scrollIntoView()),
  );
}

function insertParagraphAfter(view: EditorView, depth: number): boolean {
  const { state } = view;
  const paragraph = state.schema.nodes.paragraph;
  if (paragraph === undefined) {
    return false;
  }
  const after = state.selection.$from.after(depth);
  const tr = state.tr.insert(after, paragraph.create());
  tr.setSelection(TextSelection.create(tr.doc, after + 1));
  view.dispatch(tr.scrollIntoView());
  return true;
}
