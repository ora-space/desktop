import {
  composerFileAttrsFromUnknown,
  composerFileChipTitle,
  composerFileLabel,
} from "@ora/editor/composer";
import { IconX } from "@tabler/icons-react";
import { NodeViewWrapper, type NodeViewProps } from "@tiptap/react";
import type {
  DragEvent as ReactDragEvent,
  MouseEvent as ReactMouseEvent,
} from "react";
import { useTranslation } from "react-i18next";
import { FileRefChipContent } from "../file-ref-chip";

/**
 * Inline path mention chip: type/folder icon + basename, Cursor-style (no pill).
 * Wired through `AppComposerFile` so the explorer and @ picker share one visual.
 */
export function ComposerFileChipView({ node, editor, getPos }: NodeViewProps) {
  const { t } = useTranslation();
  const attrs = composerFileAttrsFromUnknown(node.attrs);
  const kind = attrs.kind === "directory" ? "directory" : "file";
  const title = composerFileChipTitle(attrs);

  const selectOnlyThisChip = (event: ReactMouseEvent<HTMLElement>): void => {
    event.preventDefault();
    event.stopPropagation();
    const pos = getPos();
    if (typeof pos !== "number") return;
    editor.chain().focus().setNodeSelection(pos).run();
  };

  /** Drops this reference from the prompt; hover swaps the type icon for it. */
  const removeThisChip = (event: ReactMouseEvent<HTMLElement>): void => {
    event.preventDefault();
    event.stopPropagation();
    const pos = getPos();
    if (typeof pos !== "number") return;
    editor
      .chain()
      .focus()
      .deleteRange({ from: pos, to: pos + node.nodeSize })
      .run();
  };

  return (
    <NodeViewWrapper
      as="span"
      className="composer-file-ref"
      data-composer-file={attrs.path}
      data-kind={kind}
      {...(attrs.startLine === undefined
        ? {}
        : { "data-start-line": String(attrs.startLine) })}
      {...(attrs.endLine === undefined
        ? {}
        : { "data-end-line": String(attrs.endLine) })}
      contentEditable={false}
      title={title}
      draggable={false}
      onDragStart={(event: ReactDragEvent<HTMLElement>) => {
        event.preventDefault();
      }}
      onMouseDown={(event: ReactMouseEvent<HTMLElement>) => {
        if (event.button !== 0) return;
        if (event.detail >= 2 || event.ctrlKey || event.metaKey) {
          selectOnlyThisChip(event);
        }
      }}
      onDoubleClick={selectOnlyThisChip}
    >
      {editor.isEditable && (
        <button
          type="button"
          // Chips sit in a contenteditable; a tab stop per chip would bury the
          // send button. Keyboard removal stays select-the-chip plus Backspace.
          tabIndex={-1}
          className="composer-file-ref-remove"
          aria-label={t("chat.removeFileReference", {
            name: composerFileLabel(attrs),
          })}
          onMouseDown={(event: ReactMouseEvent<HTMLElement>) => {
            // Claim the press so the editor cannot node-select the chip first.
            event.preventDefault();
            event.stopPropagation();
          }}
          onClick={removeThisChip}
        >
          <IconX className="composer-file-ref-remove-glyph" />
        </button>
      )}
      {/* Renders after the button so hover can swap them in the same slot. */}
      <FileRefChipContent attrs={attrs} />
    </NodeViewWrapper>
  );
}
