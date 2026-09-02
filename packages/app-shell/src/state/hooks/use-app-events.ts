import { useEffect, useState } from "react";
import type { ContractsClient } from "@ora/contracts";
import { useQueryClient } from "@tanstack/react-query";
import { queryKeys } from "./query-keys";

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

    const refetchSessions = () => {
      void queryClient.refetchQueries({ queryKey: queryKeys.sessions });
    };
    const invalidateSessions = () => {
      void queryClient.invalidateQueries({ queryKey: queryKeys.sessions });
    };
    // Runtime transitions, scans, and package removal can happen outside mutations on this
    // client, so refresh every view derived from installed plugin state together.
    const invalidatePluginState = () => {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.installedPlugins,
      });
      void queryClient.invalidateQueries({
        queryKey: queryKeys.agentRuntimeStatus,
      });
      void queryClient.invalidateQueries({ queryKey: queryKeys.skills });
    };
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
      refetchSessions();
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
            refetchSessions();
          } else if (event.type === "session_title_updated") {
            invalidateSessions();
          } else if (event.type === "plugin_status_changed") {
            invalidatePluginState();
          } else if (event.type === "agent_models_invalidated") {
            void queryClient.invalidateQueries({
              queryKey: queryKeys.agentModelsForAgent(event.agent_ref),
            });
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
