import { useMutation } from "@tanstack/react-query";
import { useStore } from "zustand";
import { useChatStore } from "../../chat-store-context";

/**
 * Applies one configuration selection — in practice the model — to a session.
 *
 * The request is owned by the chat store, which records both the agent's
 * authoritative answer and any failure to reach it. This wrapper exists for the
 * pending flag the picker shows while the round trip is in flight; the rejection
 * it re-raises is already on screen through the conversation's error.
 */
export function useSetSessionConfig() {
  const chatStore = useChatStore();
  const setSessionConfig = useStore(chatStore, (state) => state.setSessionConfig);
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
