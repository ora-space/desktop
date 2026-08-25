import {
  composerFileAttrsFromUnknown,
  composerFileChipTitle,
} from "@ora/editor/composer";
import { NodeViewWrapper, type NodeViewProps } from "@tiptap/react";
import type {
  DragEvent as ReactDragEvent,
  MouseEvent as ReactMouseEvent,
} from "react";
import { FileRefChipContent } from "../file-ref-chip";

/**
 * Inline path mention chip: type/folder icon + basename, Cursor-style (no pill).
 * Wired through `AppComposerFile` so the explorer and @ picker share one visual.
 */
export function ComposerFileChipView({ node, editor, getPos }: NodeViewProps) {
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
      <FileRefChipContent attrs={attrs} />
    </NodeViewWrapper>
  );
}
