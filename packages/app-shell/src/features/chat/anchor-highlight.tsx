/**
 * Navigation focus outline shared by full chat and embedded node conversations.
 *
 * Uses an inset box-shadow ring instead of an SVG stroke-dash path: percentage SVG
 * rects plus WAAPI dashoffset were rendering as partial outlines in both surfaces.
 */
export function AnchorHighlight() {
  return (
    <div
      aria-hidden="true"
      data-anchor-highlight
      className="pointer-events-none absolute inset-0 z-10 rounded-[inherit] opacity-0 shadow-[inset_0_0_0_1.5px_color-mix(in_oklch,var(--foreground)_80%,transparent)]"
    />
  );
}
