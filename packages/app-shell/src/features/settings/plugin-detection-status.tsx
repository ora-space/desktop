import { useTranslation } from "react-i18next";
import { cn } from "@ora/ui";
import { IconCheck, IconLoader2 } from "@tabler/icons-react";
import type { AgentCliStatus } from "@ora/contracts";

/**
 * Read-only status badge shown in place of the Install/Uninstall controls for the CLI
 * plugins whose installed state is derived from live backend detection (see
 * `useAgentRuntimeStatus`) rather than the manual install toggle used by every other
 * plugin in the catalog.
 */
export function PluginDetectionStatus({ status, className }: { status: AgentCliStatus; className?: string }) {
  const { t } = useTranslation();

  if (status === "starting") {
    return (
      <span className={cn("inline-flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground", className)}>
        <IconLoader2 className="size-3.5 animate-spin" />
        {t("settings.plugins.detecting")}
      </span>
    );
  }

  if (status === "ready") {
    return (
      <span className={cn("inline-flex shrink-0 items-center gap-1.5 text-xs text-muted-foreground", className)}>
        <IconCheck className="size-3.5" />
        {t("settings.plugins.detected")}
      </span>
    );
  }

  return (
    <span className={cn("shrink-0 text-xs text-muted-foreground", className)}>
      {t("settings.plugins.notDetected")}
    </span>
  );
}
