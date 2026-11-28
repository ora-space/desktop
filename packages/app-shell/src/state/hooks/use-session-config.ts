import { useMutation } from "@tanstack/react-query";
import { useStore } from "zustand";
import { useChatStore } from "../../chat-store-context";

/**
 * Applies one configuration selection — in practice the model — to a session.
 *
 * The request is owned by the chat store, which settles it once the options the
 * agent left the session on are known, and records any failure to reach it. This
 * wrapper exists for the pending flag the picker shows until then; the rejection
 * it re-raises is already on screen through the conversation's error. It resolves
 * to `noProvider` when the session had none, leaving the choice to the picker as
 * an intent.
 */
export function useSetSessionConfig() {
  const chatStore = useChatStore();
  const setSessionConfig = useStore(
    chatStore,
    (state) => state.setSessionConfig,
  );
  return useMutation({
    mutationFn: ({
      sessionId,
      configId,
      value,
    }: {
      sessionId: string;
      configId: string;
      value: string;
    }) => setSessionConfig(sessionId, configId, value),
  });
}
