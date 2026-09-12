import { IconInfoCircle } from "@tabler/icons-react";
import { Tooltip, TooltipContent, TooltipTrigger } from "@ora/ui";

/** Renders a titled usage section whose explanation is available on hover or focus. */
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
  return (
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
            />
          }
        >
          <IconInfoCircle className="size-3.5" />
        </TooltipTrigger>
        <TooltipContent className="max-w-80 space-y-1.5 leading-relaxed">
          <p>{tooltip}</p>
          <p>{details}</p>
        </TooltipContent>
      </Tooltip>
    </div>
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
