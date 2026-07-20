import { useEffect } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type { ContractsClient, Session, SessionStatus } from "@ora/contracts";
import type { ChatStore } from "@ora/chat";
import { queryKeys } from "./query-keys";

/**
 * Mirrors live ACP prompt activity onto the persisted session status.
 *
 * The chat store owns the authoritative in-flight signal but deliberately knows
 * nothing about the REST contracts, so this hook is the single seam that
 * translates that signal into `session.update` calls. Keeping the translation
 * here lets every view keep rendering `Session.status` alone.
 *
 * A stalled turn intentionally leaves the session `running`: the agent has not
 * reported that it stopped, so claiming otherwise would be a guess.
 */
export function useSessionStatusSync(client: ContractsClient, chatStore: ChatStore): void {
  const queryClient = useQueryClient();

  useEffect(() => {
    // Track what was last pushed per session so repeated store updates - one
    // per streamed chunk - do not each trigger an identical write.
    const lastPushed = new Map<string, SessionStatus>();

    return chatStore.subscribe((state) => {
      for (const [oraSessionId, conversation] of Object.entries(state.conversations)) {
        const status: SessionStatus = conversation.isResponding ? "running" : "stopped";
        if (lastPushed.get(oraSessionId) === status) continue;

        const sessions = (queryClient.getQueryData(queryKeys.sessions) as Session[] | undefined) ?? [];
        const session = sessions.find((candidate) => candidate.id === oraSessionId);
        // A conversation outlives its session row after a delete; skip it.
        if (session === undefined) continue;

        lastPushed.set(oraSessionId, status);
        if (session.status === status) continue;

        void client.session
          .update({
            sessionId: session.id,
            taskId: session.taskId,
            agentId: session.agentId,
            agentSessionId: session.agentSessionId,
            status,
          })
          .then(() => queryClient.invalidateQueries({ queryKey: queryKeys.sessions }))
          .catch(() => {
            // Forget the write so the next transition retries, rather than
            // assuming the backend accepted a status it never received.
            lastPushed.delete(oraSessionId);
          });
      }
    });
  }, [client, chatStore, queryClient]);
}
