/**
 * Whether clamped / catalog-style UI should offer a full-text preview.
 * Short singles stay static; longer or multi-line copy gets a popover.
 */
export function shouldPreviewBrief(text: string): boolean {
  const trimmed = text.trim();
  if (trimmed === "") {
    return false;
  }
  return trimmed.length > 96 || trimmed.includes("\n");
}
