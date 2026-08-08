import type { ReactNode } from "react";
import {
  Button,
  Popover,
  PopoverContent,
  PopoverDescription,
  PopoverHeader,
  PopoverTitle,
  PopoverTrigger,
  cn,
} from "@ora/ui";
import { IconInfoCircle } from "@tabler/icons-react";

interface RunBriefPopoverProps {
  title: string;
  body: string;
  openLabel: string;
  /** Compact row content (name / clamped text). */
  children: ReactNode;
  /** Optional leading icon inside the trigger (e.g. provider logo). */
  leading?: ReactNode;
  side?: "top" | "bottom" | "left" | "right" | "inline-start" | "inline-end";
  className?: string;
  /** Stop parent click handlers (theater stage card opens the inspector). */
  stopPropagation?: boolean;
}

/**
 * Shared preview affordance for catalog briefs and truncated long text.
 * Trigger stays in the same quiet field chrome as read-only inspector rows.
 */
export function RunBriefPopover({
  title,
  body,
  openLabel,
  children,
  leading,
  side = "left",
  className,
  stopPropagation = false,
}: RunBriefPopoverProps) {
  return (
    <Popover>
      <PopoverTrigger
        render={
          <Button
            type="button"
            variant="ghost"
            className={cn(
              "h-auto w-full justify-between gap-2 rounded-lg border border-border/70",
              "bg-muted/25 px-3 py-2 font-normal text-foreground/90",
              "hover:bg-muted/45 hover:text-foreground",
              "focus-visible:ring-1 focus-visible:ring-ring",
              className,
            )}
            aria-label={openLabel}
            onClick={stopPropagation
              ? (event) => event.stopPropagation()
              : undefined}
            onPointerDown={stopPropagation
              ? (event) => event.stopPropagation()
              : undefined}
          />
        }
      >
        <span className="flex min-w-0 flex-1 items-start gap-2 text-left">
          {leading}
          <span className="min-w-0 flex-1">{children}</span>
        </span>
        <IconInfoCircle
          className="mt-0.5 size-3.5 shrink-0 text-muted-foreground/70"
          aria-hidden="true"
        />
      </PopoverTrigger>
      <PopoverContent
        align="start"
        side={side}
        sideOffset={8}
        className="w-72 max-w-[min(18rem,calc(100vw-2rem))] gap-1.5 p-3"
        onClick={stopPropagation ? (event) => event.stopPropagation() : undefined}
      >
        <PopoverHeader className="gap-1">
          <PopoverTitle className="text-xs font-medium leading-4">
            {title}
          </PopoverTitle>
          <PopoverDescription className="max-h-64 overflow-y-auto whitespace-pre-wrap text-[11px] leading-5">
            {body}
          </PopoverDescription>
        </PopoverHeader>
      </PopoverContent>
    </Popover>
  );
}
