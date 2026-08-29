import type { PromptTokenKind } from "@ora/editor/composer";

interface PromptTokenChipProps {
  kind: PromptTokenKind;
  name: string;
}

/**
 * Read-only skill/command/role chip for sent-message history. Mirrors the
 * composer's `composer-mention` span so the same token looks identical before
 * and after sending. The selection rules on that class are scoped to the editor
 * shell, so outside it the chip is plain selectable text (with a visible
 * marquee highlight).
 */
export function PromptTokenChip({ kind, name }: PromptTokenChipProps) {
  const prefix = kind === "command" ? "/" : kind === "role" ? "@" : "$";
  return (
    <span className="composer-mention" data-prompt-token={kind}>
      {prefix}
      {name}
    </span>
  );
}
