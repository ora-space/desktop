import {
  composerFileAttrsFromUnknown,
  composerFileLabel,
  composerFilePlainText,
} from "@ora/editor/composer";
import { NodeViewWrapper, type NodeViewProps } from "@tiptap/react";
import { WorkspaceFileIcon } from "../files/workspace-file-visuals";

/**
 * Inline path mention chip: type/folder icon + basename, Cursor-style (no pill).
 * Wired through `AppComposerFile` so the explorer and @ picker share one visual.
 */
export function ComposerFileChipView({ node }: NodeViewProps) {
  const attrs = composerFileAttrsFromUnknown(node.attrs);
  const kind = attrs.kind === "directory" ? "directory" : "file";
  const title = composerFilePlainText(attrs).replace(/`/g, "");

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
    >
      <WorkspaceFileIcon
        path={attrs.path}
        kind={kind}
        className="composer-file-ref-icon"
      />
      <span className="composer-file-ref-label">
        {composerFileLabel(attrs)}
      </span>
    </NodeViewWrapper>
  );
}
