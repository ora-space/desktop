import type { SVGProps } from "react";

/**
 * RTK's mark, traced from the project's own site favicon
 * (`rtk-ai.app/favicon.svg`) — a lightning bolt, matching the tool's
 * token/speed-cutting positioning. The original is two-tone (green fill,
 * violet stroke); here it is a single `currentColor` fill so the plugin
 * tile's tone drives it, the same way the other traced brand marks work.
 */
export function RtkMark(props: SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox="0 0 24 24" fill="currentColor" aria-hidden="true" {...props}>
      <path d="M13 2L3 14h9l-1 8 10-12h-9l1-8z" />
    </svg>
  );
}
