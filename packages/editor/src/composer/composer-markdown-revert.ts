import { Extension } from "@tiptap/core";
import type { Mark, Node } from "@tiptap/pm/model";
import { TextSelection } from "@tiptap/pm/state";
import type { EditorView } from "@tiptap/pm/view";
import { handleComposerCodeBackspace } from "./composer-code-fence.ts";
import { inlineMarksPlainText } from "./composer-plain-text.ts";

export const MARKDOWN_REVERT_META = "composerMarkdownRevert";

/**
 * After space/input-rule conversion, Backspace against rendered mark runs
 * restores their Markdown source so further deletes remove real characters
 * (`*`, `=`, …) instead of eating styled text. Empty code fences still collapse
 * first (same key as the fence module).
 */
export const ComposerMarkdownRevert = Extension.create({
  name: "composerMarkdownRevert",

  addKeyboardShortcuts() {
    return {
      Backspace: () => {
        const { view } = this.editor;
        return (
          handleComposerCodeBackspace(view) ||
          handleComposerMarkdownBackspace(view)
        );
      },
    };
  },
});

/**
 * Reverts contiguous mark runs immediately before the caret (plus a trailing
 * confirm space). Headings and plain text after converted marks delete normally.
 */
export function handleComposerMarkdownBackspace(view: EditorView): boolean {
  const { state } = view;
  if (!state.selection.empty) {
    return false;
  }
  const { $from } = state.selection;
  const parent = $from.parent;
  if (!parent.isTextblock || parent.type.name === "codeBlock") {
    return false;
  }
  // Headings stay nodes; Backspace deletes characters instead of `# Title`.
  if (parent.type.name === "heading") {
    return false;
  }

  const range = revertRangeBeforeCaret($from);
  if (range === null) {
    return false;
  }

  const { tr } = state;
  tr.replaceWith(
    range.from,
    range.to,
    range.source.length === 0 ? [] : state.schema.text(range.source),
  );
  tr.setSelection(
    TextSelection.create(tr.doc, range.from + range.source.length),
  );
  tr.setMeta(MARKDOWN_REVERT_META, true);
  view.dispatch(tr.scrollIntoView());
  return true;
}

/**
 * Contiguous marked text touching the caret, optionally through trailing
 * whitespace left by space-confirm. Extends left across adjacent runs so
 * `**a***b*` restores together. Underline stops the range because Markdown
 * cannot round-trip it.
 */
function revertRangeBeforeCaret($from: {
  pos: number;
  nodeBefore: Node | null;
  doc: Node;
}): { from: number; to: number; source: string } | null {
  let cursor = $from.pos;
  let before = $from.nodeBefore;

  if (
    before !== null &&
    before.isText &&
    before.marks.length === 0 &&
    before.text !== undefined &&
    /^\s+$/.test(before.text)
  ) {
    cursor -= before.nodeSize;
    before = $from.doc.resolve(cursor).nodeBefore;
  }

  if (
    before === null ||
    !before.isText ||
    before.marks.length === 0 ||
    before.text === undefined ||
    hasNonMarkdownMark(before.marks)
  ) {
    return null;
  }

  let from = cursor - before.nodeSize;
  const nodes: Node[] = [before];
  while (from > 0) {
    const previous = $from.doc.resolve(from).nodeBefore;
    if (
      previous === null ||
      !previous.isText ||
      previous.marks.length === 0 ||
      previous.text === undefined ||
      hasNonMarkdownMark(previous.marks)
    ) {
      break;
    }
    nodes.unshift(previous);
    from -= previous.nodeSize;
  }

  const source = nodes
    .map((node) => inlineMarksPlainText(node.text ?? "", node.marks))
    .join("");
  const plain = nodes.map((node) => node.text ?? "").join("");
  if (source === plain) {
    return null;
  }

  return {
    from,
    to: $from.pos,
    source,
  };
}

function hasNonMarkdownMark(marks: readonly Mark[]): boolean {
  return marks.some((mark) => mark.type.name === "underline");
}
