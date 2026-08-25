import { Extension } from "@tiptap/core";
import type { Editor } from "@tiptap/core";
import { TextSelection } from "@tiptap/pm/state";
import { convertMarkdownFenceOpener } from "./composer-code-fence";

/**
 * Shift+Enter starts a new line/block the way the rest of the composer does,
 * including inside a code fence. A ```lang opener becomes a fence instead of
 * a body newline.
 */
export const ComposerNewline = Extension.create({
  name: "composerNewline",
  // Below composerCodeFence (1100) so Enter can still open a fence; above
  // composerMarkdown (50) so Shift+Enter wins over paste/input rules.
  priority: 1000,

  addKeyboardShortcuts() {
    return {
      "Shift-Enter": () => splitComposerBlock(this.editor),
    };
  },
});

function splitComposerBlock(editor: Editor): boolean {
  if (convertMarkdownFenceOpener(editor.view)) {
    return true;
  }
  if (editor.isActive("codeBlock")) {
    return editor.commands.insertContent("\n");
  }
  if (editor.isActive("listItem")) {
    const { $from } = editor.state.selection;
    if ($from.parent.content.size === 0) {
      return editor.commands.liftListItem("listItem");
    }
    return editor.commands.splitListItem("listItem");
  }
  if (editor.isActive("taskItem")) {
    const { $from } = editor.state.selection;
    if ($from.parent.content.size === 0) {
      return editor.commands.liftListItem("taskItem");
    }
    return editor.commands.splitListItem("taskItem");
  }
  const { $from } = editor.state.selection;
  if (editor.isActive("blockquote") && $from.parent.content.size === 0) {
    return editor.commands.liftEmptyBlock();
  }
  if (
    $from.parentOffset === 0 &&
    $from.parent.content.size > 0 &&
    $from.parent.type.isTextblock
  ) {
    return insertEmptyBlockBefore(editor);
  }
  return editor.commands.splitBlock();
}

/**
 * Caret at the start of a block: insert an empty paragraph above so the
 * existing text moves down instead of staying on the first line.
 */
function insertEmptyBlockBefore(editor: Editor): boolean {
  return editor.commands.command(({ state, tr, dispatch }) => {
    const paragraph = state.schema.nodes.paragraph;
    const { $from } = state.selection;
    if (paragraph === undefined || !$from.parent.type.isTextblock) {
      return false;
    }
    if (dispatch) {
      const insertPos = $from.before($from.depth);
      tr.insert(insertPos, paragraph.create());
      tr.setSelection(TextSelection.create(tr.doc, insertPos + 1));
    }
    return true;
  });
}
