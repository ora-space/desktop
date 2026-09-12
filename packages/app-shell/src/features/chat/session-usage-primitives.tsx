import { useState } from "react";
import { IconInfoCircle } from "@tabler/icons-react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@ora/ui";

/** Renders a titled usage section whose explanation works with pointer, keyboard, and touch. */
export function UsageHeading({
  id,
  title,
  tooltip,
  details,
}: {
  id: string;
  title: string;
  tooltip: string;
  details: string;
}) {
  const [expanded, setExpanded] = useState(false);
  return (
    <>
      <div className="flex items-center gap-1">
        <h3 id={id} className="text-sm font-medium">
          {title}
        </h3>
        <Tooltip>
          <TooltipTrigger
            render={
              <button
                type="button"
                className="rounded-sm text-muted-foreground outline-none hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring"
                aria-label={tooltip}
                aria-expanded={expanded}
                onClick={() => setExpanded((value) => !value)}
              />
            }
          >
            <IconInfoCircle className="size-3.5" />
          </TooltipTrigger>
          <TooltipContent>{tooltip}</TooltipContent>
        </Tooltip>
      </div>
      {expanded && (
        <p className="rounded-md bg-muted/60 p-2 text-[11px] leading-relaxed text-muted-foreground">
          {details}
        </p>
      )}
    </>
  );
}

/** Renders one compact labeled value in the usage details grid. */
export function UsageMetric({
  label,
  value,
}: {
  label: string;
  value: string;
}) {
  return (
    <div className="rounded-md bg-muted/50 px-2 py-1.5">
      <div className="text-[10px] text-muted-foreground">{label}</div>
      <div className="font-medium tabular-nums">{value}</div>
    </div>
  );
}
