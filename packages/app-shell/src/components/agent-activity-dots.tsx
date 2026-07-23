/**
 * Row-position animation for each cell of the 3x3 grid, in row-major order.
 *
 * Spelled out as whole class names because Tailwind scans source text: a name
 * assembled at runtime would never make it into the generated stylesheet.
 */
const AGENT_DOT_ANIMATIONS = [
  "animate-dot-column-top", "animate-dot-column-top", "animate-dot-column-top",
  "animate-dot-column-middle", "animate-dot-column-middle", "animate-dot-column-middle",
  "animate-dot-column-bottom", "animate-dot-column-bottom", "animate-dot-column-bottom",
];

/**
 * Offset between columns, one third of the 1.2s `dot-column-*` cycle.
 *
 * Those keyframes hold at the top for this long plus 60ms, which is what makes
 * a column start falling a beat after the column to its right arrives. Retiming
 * the animation means moving this and the cycle duration together, or the
 * handoff breaks instead of just running at a different speed.
 */
const AGENT_DOT_COLUMN_DELAY_MS = 400;

interface AgentActivityDotsProps {
  label: string;
  /** Extra classes for the grid wrapper — commonly the text colour to tint the dots. */
  className?: string;
  /** Size class for each square; defaults to the sidebar's 3px dots. */
  dotClassName?: string;
}

/**
 * Marks a working agent with a 3x3 grid of squares.
 *
 * Every column runs the same two-dot window that climbs to the top, pauses,
 * and drops back down; columns are offset from each other so the three never
 * move in lockstep. The dots inherit `currentColor`, so callers set the tint
 * through the wrapper's text colour.
 *
 * The animation carries the "still running" meaning on its own, so callers show
 * nothing at all when the agent is idle rather than a stalled grid.
 */
export function AgentActivityDots({ label, className, dotClassName = "size-[3px]" }: AgentActivityDotsProps) {
  return (
    <span role="img" aria-label={label} className={`grid grid-cols-3 gap-[2.5px] ${className ?? ""}`}>
      {AGENT_DOT_ANIMATIONS.map((animation, index) => (
        <span
          key={index}
          className={`${dotClassName} rounded-[0.5px] bg-current ${animation}`}
          style={{ animationDelay: `${(index % 3) * AGENT_DOT_COLUMN_DELAY_MS}ms` }}
        />
      ))}
    </span>
  );
}
