import { useTranslation } from "react-i18next";
import { Badge, cn, Tooltip, TooltipContent, TooltipTrigger } from "@ora/ui";

/**
 * Where a plugin (and the Skills it ships) comes from, as shown to the user.
 *
 * The namespace is identity, not display text (registry ADR 20260831 D6), so a marketplace origin
 * carries a separate human-readable `label`.
 */
export type PluginOriginDisplay =
  { kind: "official" } | { kind: "marketplace"; label: string };

/**
 * A small badge naming a plugin's origin, with a tooltip explaining what that origin means.
 *
 * The tooltip deliberately states only the kind of source and its consequence — never the full
 * URL or the domain — so the badge stays short and the tooltip stays narrow.
 */
export function OriginTag({
  origin,
  className,
}: {
  origin: PluginOriginDisplay;
  className?: string;
}) {
  const { t } = useTranslation();
  const text =
    origin.kind === "official" ? t("plugins.origin.official") : origin.label;
  const description =
    origin.kind === "official"
      ? t("plugins.origin.officialDescription")
      : t("plugins.origin.marketplaceDescription");

  return (
    <Tooltip>
      <TooltipTrigger
        render={
          <Badge
            variant="outline"
            data-slot="origin-tag"
            className={cn("font-normal text-muted-foreground", className)}
          />
        }
      >
        <span className="max-w-40 truncate">{text}</span>
      </TooltipTrigger>
      <TooltipContent className="max-w-64">{description}</TooltipContent>
    </Tooltip>
  );
}
