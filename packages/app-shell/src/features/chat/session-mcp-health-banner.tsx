import type {
  McpHealthEntry,
  Session,
  SessionMcpSelection,
} from "@ora/contracts";
import { useTranslation } from "react-i18next";
import { Button } from "@ora/ui";
import { IconAlertTriangle } from "@tabler/icons-react";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";
import { useMcpHealth } from "../../state/hooks/use-mcp-health";
import { useWorkspaceCwd } from "../../state/hooks/use-workspace-cwd";
import { useUiStore } from "../../state/stores/ui-store";

/**
 * Warns, without blocking, that the Host could not reach an MCP this Session is allowed to use.
 *
 * The banner is deliberately narrower than the plugin card: it lists only members of this
 * Session's Effective MCP Set, so an MCP the Session never selected stays invisible here even
 * when the card reports it unhealthy. It also never renders `Unknown`: a probe that has not
 * settled, or a card that has no real workspace directory, is not a failure and must not look
 * like one. Prompts keep flowing — this is Host-observed health, not a delivery gate.
 */
export function SessionMcpHealthBanner({
  session,
}: {
  session: Session | undefined;
}) {
  const { t } = useTranslation();
  // A Session's MCP view is bound to its workspace directory; without one the card view would
  // answer a different question, so the banner waits instead of querying with `null`.
  const cwd = useWorkspaceCwd(session?.workspaceId).data;
  const health = useMcpHealth(cwd);
  const plugins = useInstalledPlugins();
  const openPluginSettings = useUiStore((state) => state.openPluginSettings);

  if (session === undefined || health.data === undefined) return null;
  const inSession = selectionFilter(session.mcpSelection);
  const unhealthy = health.data.filter(
    (entry) =>
      inSession(entry.identity.pluginId) && entry.status.status === "unhealthy",
  );
  if (unhealthy.length === 0) return null;

  const displayName = (pluginId: string) =>
    plugins.data?.find((plugin) => plugin.id === pluginId)?.displayName ??
    pluginId;

  return (
    <div
      role="status"
      data-mcp-health-banner="unhealthy"
      className="mx-3 mb-2 flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs sm:mx-4"
    >
      <IconAlertTriangle
        className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400"
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1">
        <p className="font-medium text-amber-700 dark:text-amber-300">
          {t("chat.mcpHealth.title")}
        </p>
        {unhealthy.map((entry) => (
          <p
            key={`${entry.identity.pluginId}:${entry.identity.cwd ?? ""}`}
            data-mcp-health-code={
              entry.status.status === "unhealthy"
                ? entry.status.error_code
                : undefined
            }
            className="mt-0.5 flex flex-wrap items-center gap-x-2 break-words text-muted-foreground"
          >
            <span>{mcpUnhealthyLine(entry, displayName, t)}</span>
            <Button
              variant="ghost"
              size="sm"
              className="h-5 px-1.5 text-[11px]"
              onClick={() =>
                openPluginSettings({
                  kind: "configure",
                  pluginId: entry.identity.pluginId,
                  displayName: displayName(entry.identity.pluginId),
                })
              }
            >
              {t("chat.mcpHealth.configure")}
            </Button>
          </p>
        ))}
      </div>
    </div>
  );
}

/**
 * Decides whether one plugin may appear in this Session's MCP banner.
 *
 * Ordinary chats resolve every currently eligible MCP, while a workflow Session shows only the
 * whitelist its node froze — an unselected plugin must not surface here even when its card is
 * unhealthy.
 */
function selectionFilter(
  selection: SessionMcpSelection,
): (pluginId: string) => boolean {
  return selection.mode === "automatic"
    ? () => true
    : (pluginId) => selection.pluginIds.includes(pluginId);
}

/** Renders one unavailable member with its stable code, never with raw server output. */
function mcpUnhealthyLine(
  entry: McpHealthEntry,
  displayName: (pluginId: string) => string,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  const code =
    entry.status.status === "unhealthy" ? entry.status.error_code : "";
  return t("chat.mcpHealth.member", {
    name: displayName(entry.identity.pluginId),
    code,
  });
}
