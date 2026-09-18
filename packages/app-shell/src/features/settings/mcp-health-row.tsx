import type { McpHealthErrorCode, McpHealthStatus } from "@ora/contracts";
import { useTranslation } from "react-i18next";
import { Button } from "@ora/ui";
import {
  IconAlertTriangle,
  IconCheck,
  IconLoader2,
  IconRefresh,
} from "@tabler/icons-react";
import { useContractErrorToast } from "../../i18n/use-contract-error-toast";
import {
  useMcpHealth,
  useProbeMcpHealth,
} from "../../state/hooks/use-mcp-health";

/**
 * The plugin card's third, independent status line: Host-observed MCP health.
 *
 * It sits beside install state and configuration completeness instead of being merged into them,
 * because a healthy handshake is not delivery success and a failed one is not a configuration
 * problem. Nothing here is named or worded as ready-in-session: the Host only knows whether it
 * could complete `initialize` and `tools/list` against this binding product right now.
 *
 * The line is rendered only for an MCP whose configuration is complete, so a member that is not
 * yet eligible keeps showing exactly the existing configuration state and no health row.
 */
export function McpHealthRow({ pluginId }: { pluginId: string }) {
  const { t } = useTranslation();
  const showContractError = useContractErrorToast();
  const health = useMcpHealth(null);
  const probe = useProbeMcpHealth(pluginId, null);
  const entry = health.data?.find(
    (candidate) => candidate.identity.pluginId === pluginId,
  );

  if (entry === undefined) return null;
  const status = entry.status;
  const failed = status.status === "unhealthy";
  const probing = probe.isPending;

  return (
    <span
      data-slot="mcp-health"
      data-mcp-health={status.status}
      data-mcp-health-code={failed ? status.error_code : undefined}
      className="mt-1 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px]"
    >
      {failed ? (
        <IconAlertTriangle
          className="size-3.5 shrink-0 text-amber-600 dark:text-amber-400"
          aria-hidden="true"
        />
      ) : status.status === "healthy" ? (
        <IconCheck
          className="size-3.5 shrink-0 text-emerald-600 dark:text-emerald-400"
          aria-hidden="true"
        />
      ) : null}
      <span className={healthTextClass(status)}>{healthLabel(status, t)}</span>
      {status.status === "healthy" && (
        // The distinction users must not lose: this probe ran on the Host, not inside a Session.
        <span className="text-muted-foreground">
          {t("settings.plugins.mcpHealth.healthyHint")}
        </span>
      )}
      {status.status === "unknown" && status.reason === "context_missing" && (
        // Re-detecting here would have to invent a workspace directory, so the card explains the
        // limitation instead of offering an action it cannot honour.
        <span className="text-muted-foreground">
          {t("settings.plugins.mcpHealth.contextMissingHint")}
        </span>
      )}
      {canReDetect(status) && (
        <Button
          variant="ghost"
          size="sm"
          className="h-5 gap-1 px-1.5 text-[11px] text-muted-foreground"
          disabled={probing}
          aria-label={t("settings.plugins.mcpHealth.reDetect")}
          onClick={() =>
            probe.mutate(undefined, {
              onError: (cause) =>
                showContractError(
                  cause,
                  t("settings.plugins.mcpHealth.probeFailed"),
                ),
            })
          }
        >
          {probing ? (
            <IconLoader2 className="size-3 animate-spin" />
          ) : (
            <IconRefresh className="size-3" />
          )}
          {t(
            probing
              ? "settings.plugins.mcpHealth.reDetecting"
              : "settings.plugins.mcpHealth.reDetect",
          )}
        </Button>
      )}
    </span>
  );
}

/** Re-detecting is possible only when a real probe can run for this identity. */
function canReDetect(status: McpHealthStatus): boolean {
  return !(status.status === "unknown" && status.reason === "context_missing");
}

function healthTextClass(status: McpHealthStatus): string {
  switch (status.status) {
    case "healthy":
      return "text-emerald-700 dark:text-emerald-300";
    case "unhealthy":
      return "text-amber-700 dark:text-amber-300";
    case "unknown":
      return "text-muted-foreground";
  }
}

/** Localizes one health outcome without ever claiming it reached a Session. */
function healthLabel(
  status: McpHealthStatus,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (status.status) {
    case "healthy":
      return t("settings.plugins.mcpHealth.healthy");
    case "unhealthy":
      return t("settings.plugins.mcpHealth.unhealthy", {
        reason: t(errorCodeKey(status.error_code)),
      });
    case "unknown":
      return t(
        status.reason === "not_probed"
          ? "settings.plugins.mcpHealth.notProbed"
          : "settings.plugins.mcpHealth.contextMissing",
      );
  }
}

/** One translation key per closed code keeps the vocabulary exhaustive and typo-proof. */
function errorCodeKey(code: McpHealthErrorCode): string {
  const keys: Record<McpHealthErrorCode, string> = {
    mcp_spawn_failed: "settings.plugins.mcpHealth.code.mcpSpawnFailed",
    mcp_exited_prematurely:
      "settings.plugins.mcpHealth.code.mcpExitedPrematurely",
    mcp_handshake_failed: "settings.plugins.mcpHealth.code.mcpHandshakeFailed",
    mcp_probe_timeout: "settings.plugins.mcpHealth.code.mcpProbeTimeout",
    mcp_tools_unavailable:
      "settings.plugins.mcpHealth.code.mcpToolsUnavailable",
    mcp_http_unreachable: "settings.plugins.mcpHealth.code.mcpHttpUnreachable",
    mcp_http_unauthorized:
      "settings.plugins.mcpHealth.code.mcpHttpUnauthorized",
    mcp_http_server_error: "settings.plugins.mcpHealth.code.mcpHttpServerError",
  };
  return keys[code];
}
