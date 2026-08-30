import { useTranslation } from "react-i18next";
import { Button, cn } from "@ora/ui";
import type { McpApplicationStateDto } from "@ora/contracts";
import type { McpApplicationStateController } from "../../state/hooks/use-mcp-application-state";
import { SettingsHeading } from "./settings-heading";

/**
 * Per-state presentation: a dot colour, a label tone, and the i18n keys for the
 * label plus a one-line explanation.
 *
 * Ordered to mirror the reconcile loop's precedence: amber for "configure me",
 * blue (animated) for "still working", emerald for "usable", destructive for
 * "give up and surface why".
 */
const STATE_META: Record<
  McpApplicationStateDto,
  { dot: string; tone: string; label: string; description: string }
> = {
  needs_configuration: {
    dot: "bg-amber-500",
    tone: "text-amber-600 dark:text-amber-400",
    label: "settings.mcp.stateNeedsConfiguration",
    description: "settings.mcp.stateNeedsConfigurationDescription",
  },
  waiting_for_agent: {
    dot: "bg-blue-500",
    tone: "text-blue-600 dark:text-blue-400",
    label: "settings.mcp.stateWaitingForAgent",
    description: "settings.mcp.stateWaitingForAgentDescription",
  },
  applying: {
    dot: "bg-blue-500 animate-pulse",
    tone: "text-blue-600 dark:text-blue-400",
    label: "settings.mcp.stateApplying",
    description: "settings.mcp.stateApplyingDescription",
  },
  ready: {
    dot: "bg-emerald-500",
    tone: "text-emerald-600 dark:text-emerald-400",
    label: "settings.mcp.stateReady",
    description: "settings.mcp.stateReadyDescription",
  },
  failed: {
    dot: "bg-destructive",
    tone: "text-destructive",
    label: "settings.mcp.stateFailed",
    description: "settings.mcp.stateFailedDescription",
  },
};

/** Shows the live MCP Application State for the active workspace's OpenCode surface. */
export function McpSettings({
  controller,
}: {
  controller: McpApplicationStateController;
}) {
  const { t } = useTranslation();
  const { workspaceId, state, isLoading, error, refetch } = controller;

  return (
    <div className="space-y-6">
      <SettingsHeading
        title={t("settings.mcp.title")}
        description={t("settings.mcp.description")}
      />
      {workspaceId === null ? (
        <p className="text-sm text-muted-foreground">
          {t("settings.mcp.noWorkspace")}
        </p>
      ) : state !== undefined ? (
        <McpStateBadge state={state} />
      ) : isLoading ? (
        <span role="status" className="text-xs text-muted-foreground">
          {t("settings.mcp.loading")}
        </span>
      ) : error !== null ? (
        <div className="flex items-center gap-3">
          <span role="alert" className="text-xs text-destructive">
            {t("settings.mcp.loadError")}
          </span>
          <Button variant="outline" onClick={() => refetch()}>
            {t("common.retry")}
          </Button>
        </div>
      ) : null}
    </div>
  );
}

/** A coloured dot with the state label and a one-line explanation of what it means. */
function McpStateBadge({ state }: { state: McpApplicationStateDto }) {
  const { t } = useTranslation();
  const meta = STATE_META[state];
  return (
    <div className="flex items-start gap-3">
      <span
        aria-hidden="true"
        className={cn("mt-1 size-2.5 shrink-0 rounded-full", meta.dot)}
      />
      <div className="space-y-1">
        <p className={cn("text-sm font-medium", meta.tone)}>{t(meta.label)}</p>
        <p className="text-xs leading-5 text-muted-foreground">
          {t(meta.description)}
        </p>
      </div>
    </div>
  );
}
