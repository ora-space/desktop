import { useEffect, useState } from "react";
import type { ChatStore } from "@ora/chat";
import type { ContractsClient } from "@ora/contracts";
import { useQueryClient } from "@tanstack/react-query";
import { invalidateSessions, refetchSessions } from "../data/sessions";
import { invalidatePluginState } from "../data/plugin-lifecycle";
import {
  invalidateAgentCommands,
  invalidateAllAgentCommands,
} from "../data/agent-runtime";
import { invalidateMcpHealth } from "../data/mcp-health";
import { invalidateWorkflowRun } from "../data/workflow-runs";
import { surfaceScheduledTaskEvent } from "../data/scheduled-task-events";
import {
  markScheduledTaskHistorySeen,
  surfaceStartupScheduledFacts,
} from "../data/scheduled-task-startup-surface";
import { useOptionalPlatform } from "../../platform/use-platform";

const INITIAL_RECONNECT_DELAY_MS = 1_000;
const MAX_RECONNECT_DELAY_MS = 30_000;

/**
 * Maintains the application stream and invalidates authoritative session state on loss.
 *
 * `chatStore` is told which sessions' options may have changed, and re-reads those on screen each
 * time the stream is (re)subscribed, because announcements made meanwhile are lost.
 */
export function useAppEvents(client: ContractsClient, chatStore?: ChatStore) {
  const queryClient = useQueryClient();
  const platform = useOptionalPlatform();
  const [ready, setReady] = useState(false);

  useEffect(() => {
    const controller = new AbortController();
    let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
    let reconnectDelay = INITIAL_RECONNECT_DELAY_MS;
    let disposed = false;
    let wasDisconnected = false;

    const scheduleReconnect = () => {
      if (disposed) return;
      reconnectTimer = setTimeout(() => {
        reconnectTimer = undefined;
        void consume();
      }, reconnectDelay);
      reconnectDelay = Math.min(reconnectDelay * 2, MAX_RECONNECT_DELAY_MS);
    };
    const handleDisconnect = () => {
      setReady(false);
      wasDisconnected = true;
      void refetchSessions(queryClient);
      scheduleReconnect();
    };
    const consume = async (): Promise<void> => {
      if (disposed) return;
      try {
        const events = client.appEvents.watch(
          {},
          { signal: controller.signal },
        );
        for await (const event of events) {
          if (disposed) return;
          if (event.type === "ready") {
            reconnectDelay = INITIAL_RECONNECT_DELAY_MS;
            setReady(true);
            // The initial refetch closes the gap between database changes and stream subscription.
            void refetchSessions(queryClient);
            // Skills may have changed while the stream was down.
            if (wasDisconnected) void invalidateAllAgentCommands(queryClient);
            chatStore?.getState().resyncOptions();
            wasDisconnected = false;
            // Facts recorded while no subscriber existed (startup recovery of missed
            // occurrences and interrupted runs) live in persistence; surface them once.
            void surfaceStartupScheduledFacts(client, queryClient, platform);
          } else if (event.type === "session_title_updated") {
            void invalidateSessions(queryClient);
          } else if (event.type === "session_options_changed") {
            chatStore?.getState().optionsChanged(event.session_id);
          } else if (event.type === "plugin_status_changed") {
            invalidatePluginState(queryClient);
          } else if (event.type === "mcp_health_changed") {
            // The event names only the plugin; the health views stay the single source of truth
            // and are re-queried. A probe result is never carried in the event itself.
            void invalidateMcpHealth(queryClient);
          } else if (event.type === "agent_models_invalidated") {
            void invalidateAgentCommands(queryClient, event.agent_ref);
          } else if (event.type === "workflow_run_invalidated") {
            // The event names only which run changed; the run rows stay the single source of
            // truth and are re-queried here.
            invalidateWorkflowRun(queryClient, event.run_id);
          } else if (event.type === "scheduled_task") {
            // The event announces a persisted fact; navigation and OS notifications are
            // handled together with the history invalidation. Persist the dedupe baseline
            // before showing durable failure or missed facts, so a stream reconnect cannot
            // replay the same fact through the startup recovery surface.
            void (async () => {
              if (
                event.kind === "occurrence_missed" ||
                (event.kind === "run_finished" && event.failureReason !== null)
              ) {
                const shouldSurface = await markScheduledTaskHistorySeen(
                  client,
                  queryClient,
                  event,
                );
                if (!shouldSurface) return;
              }
              await surfaceScheduledTaskEvent(
                platform,
                event,
                queryClient,
                client,
              );
            })();
          }
        }
        handleDisconnect();
      } catch {
        if (disposed || controller.signal.aborted) return;
        handleDisconnect();
      }
    };

    void consume();
    return () => {
      disposed = true;
      controller.abort();
      if (reconnectTimer !== undefined) clearTimeout(reconnectTimer);
    };
  }, [client, queryClient, platform, chatStore]);

  return { ready };
}
