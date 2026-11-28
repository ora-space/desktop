import { useTranslation } from "react-i18next";
import {
  Badge,
  badgeVariants,
  cn,
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from "@ora/ui";
import { originCopy, type OriginDisplay } from "./origin-display";

/**
 * A small badge naming an origin, with a tooltip explaining what that origin means.
 *
 * The tooltip deliberately states only the kind of source and its consequence — never the full
 * URL or the domain — so the badge stays short and the tooltip stays narrow. With `action` the
 * badge becomes a button, which the Skills page uses to open the owning plugin.
 */
export function OriginTag({
  origin,
  className,
  action,
}: {
  origin: OriginDisplay;
  className?: string;
  /** Makes the badge a button; `label` names what activating it does. */
  action?: { label: string; onActivate: () => void };
}) {
  const { t } = useTranslation();
  const { text, description } = originCopy(origin, t);
  const badgeClassName = cn("font-normal text-muted-foreground", className);
  // A truncated label stays readable in full outside the tooltip, which only explains the kind.
  const label = (
    <span className="max-w-40 truncate" title={text}>
      {text}
    </span>
  );
  const tooltip = (
    <TooltipContent className="max-w-64">{description}</TooltipContent>
  );

  if (action !== undefined) {
    // The trigger is a native button already, so it only borrows the badge's look.
    return (
      <Tooltip>
        <TooltipTrigger
          data-slot="origin-tag"
          aria-label={`${text} · ${action.label}`}
          className={cn(
            badgeVariants({ variant: "outline" }),
            badgeClassName,
            "cursor-pointer outline-none hover:bg-accent focus-visible:ring-2 focus-visible:ring-ring",
          )}
          onClick={(event) => {
            event.stopPropagation();
            action.onActivate();
          }}
        >
          {label}
        </TooltipTrigger>
        {tooltip}
      </Tooltip>
    );
  }

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Badge
            variant="outline"
            data-slot="origin-tag"
            className={badgeClassName}
          />
        }
      >
        {label}
      </TooltipTrigger>
      {tooltip}
    </Tooltip>
  );
}
