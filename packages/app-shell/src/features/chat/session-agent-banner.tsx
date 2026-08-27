import { useTranslation } from "react-i18next";
import type { InstalledPlugin, Session } from "@ora/contracts";
import { IconAlertTriangle } from "@tabler/icons-react";
import { useAgentRuntimeStatus } from "../../state/hooks/use-agent-runtime-status";
import { useInstalledPlugins } from "../../state/hooks/use-installed-plugins";

/** Why the agent a session is bound to cannot serve it right now. */
type AgentAvailability =
  | { kind: "available" }
  | { kind: "failed"; plugin: InstalledPlugin; reason: string }
  | { kind: "uninstalled" };

/**
 * Warns when the plugin behind a session's agent failed or was uninstalled.
 *
 * Rendering nothing while the agent is fine keeps callers free to mount this
 * unconditionally beside the conversation it belongs to.
 */
export function SessionAgentBanner({
  session,
}: {
  session: Session | undefined;
}) {
  const { t } = useTranslation();
  const plugins = useInstalledPlugins();
  const supervised = useAgentRuntimeStatus();
  const availability = resolveAvailability(
    session,
    plugins.data,
    supervised.data?.map((status) => status.agentRef),
  );

  if (availability.kind === "available") return null;
  return <AgentUnavailableBanner availability={availability} t={t} />;
}

/**
 * Renders one plugin-agent unavailability reason.
 */
function AgentUnavailableBanner({
  availability,
  t,
}: {
  availability: Exclude<AgentAvailability, { kind: "available" }>;
  t: ReturnType<typeof useTranslation>["t"];
}) {
  return (
    <div
      role="alert"
      // Names the reason without depending on the localized sentence, which is
      // what lets a test — or a bug report — tell the three cases apart.
      data-agent-availability={availability.kind}
      className="mx-3 mb-2 flex items-start gap-2 rounded-md border border-amber-500/30 bg-amber-500/5 px-3 py-2 text-xs sm:mx-4"
    >
      <IconAlertTriangle
        className="mt-0.5 size-4 shrink-0 text-amber-600 dark:text-amber-400"
        aria-hidden="true"
      />
      <div className="min-w-0 flex-1">
        <p className="font-medium text-amber-700 dark:text-amber-300">
          {t("chat.agentUnavailable.title")}
        </p>
        <p className="mt-0.5 break-words text-muted-foreground">
          {availability.kind === "uninstalled" &&
            t("chat.agentUnavailable.uninstalled")}
          {/* The backend's reason is the only description of what actually broke,
              so it is shown verbatim rather than flattened into one sentence. */}
          {availability.kind === "failed" &&
            t("chat.agentUnavailable.failed", {
              plugin: availability.plugin.displayName,
              reason: availability.reason,
            })}
        </p>
      </div>
    </div>
  );
}

/**
 * Decides whether a session's agent is currently servable, and why it is not.
 *
 * An agent package supplies exactly one agent under its plugin name (the
 * `name` segment of its id), which is the same value a session persists as its
 * binding, so the two are matched directly; ui packages contribute no agent.
 * An identity no plugin claims is only reported as uninstalled when the runtime
 * does not supervise it either — a built-in CLI has no plugin row and must not
 * be mistaken for a package that was removed.
 *
 * Both queries answer `undefined` until they resolve; treating that as available
 * keeps the banner from flashing on every mount before anything is known.
 */
function resolveAvailability(
  session: Session | undefined,
  plugins: InstalledPlugin[] | undefined,
  supervisedAgentRefs: string[] | undefined,
): AgentAvailability {
  if (session === undefined || plugins === undefined) {
    return { kind: "available" };
  }
  const plugin = plugins.find(
    (installed) =>
      installed.kind === "agent" && installed.name === session.agentRef,
  );
  if (plugin === undefined) {
    if (
      supervisedAgentRefs === undefined ||
      supervisedAgentRefs.includes(session.agentRef)
    ) {
      return { kind: "available" };
    }
    return { kind: "uninstalled" };
  }
  if (plugin.runtime === "failed") {
    return { kind: "failed", plugin, reason: plugin.failureReason };
  }
  return { kind: "available" };
}
