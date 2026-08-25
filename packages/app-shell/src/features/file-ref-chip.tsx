import {
  composerFileChipTitle,
  composerFileLabel,
  composerFileLineRangeLabel,
  type ComposerFileAttrs,
} from "@ora/editor/composer";
import { WorkspaceFileIcon } from "./files/workspace-file-visuals";
import "./file-ref-chip.css";

/**
 * Icon + basename + optional `L12-34` range: the inside of a workspace file
 * reference chip. Split from the wrappers so the composer's TipTap node view
 * and read-only chat history render byte-identical chip innards instead of two
 * drifting copies.
 */
export function FileRefChipContent({ attrs }: { attrs: ComposerFileAttrs }) {
  const kind = attrs.kind === "directory" ? "directory" : "file";
  const rangeLabel = composerFileLineRangeLabel(attrs);
  return (
    <>
      <WorkspaceFileIcon
        path={attrs.path}
        kind={kind}
        className="composer-file-ref-icon"
      />
      <span className="composer-file-ref-label">
        <span className="composer-file-ref-name">
          {composerFileLabel(attrs)}
        </span>
        {rangeLabel !== null && (
          <span className="composer-file-ref-range">{rangeLabel}</span>
        )}
      </span>
    </>
  );
}

/**
 * Read-only chip for surfaces that only hold the sent prompt text. The composer
 * wraps the same content in a `NodeViewWrapper` instead, because an editable
 * chip also carries drag and node-selection behaviour.
 */
export function FileRefChip({ attrs }: { attrs: ComposerFileAttrs }) {
  const kind = attrs.kind === "directory" ? "directory" : "file";
  return (
    <span
      className="composer-file-ref"
      data-composer-file={attrs.path}
      data-kind={kind}
      {...(attrs.startLine === undefined
        ? {}
        : { "data-start-line": String(attrs.startLine) })}
      {...(attrs.endLine === undefined
        ? {}
        : { "data-end-line": String(attrs.endLine) })}
      title={composerFileChipTitle(attrs)}
    >
      <FileRefChipContent attrs={attrs} />
    </span>
  );
}
