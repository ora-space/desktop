import { useEffect, useState } from "react";
import type { ContractsClient } from "@ora/contracts";
import { useQueryClient } from "@tanstack/react-query";
import { refetchSessions, invalidateSessions } from "../data/sessions";
import { invalidatePluginState } from "../data/plugin-lifecycle";
import { invalidateAgentModels } from "../data/agent-runtime";

const INITIAL_RECONNECT_DELAY_MS = 1_000;
const MAX_RECONNECT_DELAY_MS = 30_000;

/** Maintains the application stream and invalidates authoritative session state on loss. */
export function useAppEvents(client: ContractsClient) {
  const queryClient = useQueryClient();
  const [ready, setReady] = useState(false);

  useEffect(() => {
    const controller = new AbortController();
    let reconnectTimer: ReturnType<typeof setTimeout> | undefined;
    let reconnectDelay = INITIAL_RECONNECT_DELAY_MS;
    let disposed = false;

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
          } else if (event.type === "session_title_updated") {
            void invalidateSessions(queryClient);
          } else if (event.type === "plugin_status_changed") {
            invalidatePluginState(queryClient);
          } else if (event.type === "agent_models_invalidated") {
            void invalidateAgentModels(queryClient, event.agent_ref);
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
  }, [client, queryClient]);

  return { ready };
}
