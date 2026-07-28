import { useMutation } from "@tanstack/react-query";
import { useStore } from "zustand";
import { useContractsClient } from "../../contracts-client-context";
import { useChatStore } from "../../chat-store-context";

/**
 * Applies one configuration selection — in practice the model — to a session.
 *
 * The agent's reply is authoritative rather than the requested value: an agent
 * that adjusted or rejected the choice describes the result, and the picker
 * renders that. Works on a warm session as well as a persisted one, so a model
 * can be chosen before the first message is sent.
 */
export function useSetSessionConfig() {
  const client = useContractsClient();
  const chatStore = useChatStore();
  const setConfigOptions = useStore(chatStore, (state) => state.setConfigOptions);
  return useMutation({
    mutationFn: ({
      sessionId,
      configId,
      value,
    }: {
      sessionId: string;
      configId: string;
      value: string;
    }) =>
      client.session
        .setConfig({ sessionId, configId, value })
        .then((response) => ({ sessionId, configOptions: response.configOptions })),
    onSuccess: ({ sessionId, configOptions }) => {
      setConfigOptions(sessionId, configOptions);
    },
  });
}
