/** Supplies the animated outline used by both user and Agent navigation targets. */
export function AnchorHighlight() {
  return (
    <svg aria-hidden="true" className="pointer-events-none absolute inset-0 size-full overflow-visible text-foreground/80">
      <rect
        data-anchor-highlight
        x="0"
        y="0"
        width="100%"
        height="100%"
        rx="12"
        pathLength="1"
        fill="none"
        stroke="currentColor"
        strokeWidth="1.5"
        strokeLinecap="round"
        strokeDasharray="1"
        strokeDashoffset="1"
        opacity="0"
        vectorEffect="non-scaling-stroke"
      />
    </svg>
  );
}
