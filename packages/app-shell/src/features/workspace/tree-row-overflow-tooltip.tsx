import type { ReactElement } from "react";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@ora/ui";

/** Cursor-style pause before the full sidebar title appears beside the row. */
export const TREE_ROW_OVERFLOW_TOOLTIP_DELAY_MS = 400;

/**
 * Shows the untruncated label in a right-side tooltip so ellipsized session
 * (and other tree) titles stay readable without changing the row layout.
 *
 * The trigger must be the same node tests and users hover (the row button),
 * because pointer events on a parent do not open a nested trigger.
 */
export function TreeRowOverflowTooltip({
  text,
  children,
}: {
  text: string;
  children: ReactElement;
}) {
  return (
    <TooltipProvider delay={TREE_ROW_OVERFLOW_TOOLTIP_DELAY_MS}>
      <Tooltip>
        <TooltipTrigger render={children} />
        <TooltipContent
          side="right"
          sideOffset={8}
          className="max-w-xs whitespace-normal break-words text-left"
        >
          {text}
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}
