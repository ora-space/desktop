import type { PromptSessionEvent, PromptSessionRequest } from "@ora/contracts";
import type { TestHandlers } from "./contracts-transport";

/**
 * Supplies scripted session handlers while preserving production client request construction.
 *
 * History completes immediately, while each prompt is recorded before its scripted
 * events are yielded. Tests can therefore pause a response at an exact event boundary
 * and exercise the UI while the turn is genuinely streaming.
 */
export function createScriptedChatSession(
  script: (request: PromptSessionRequest) => AsyncIterable<PromptSessionEvent>,
) {
  const promptRequests: PromptSessionRequest[] = [];

  return {
    promptRequests,
    handlers: {
      loadSession: async function* () {
        yield { type: "completed" };
      },
      promptSession: async function* (request) {
        promptRequests.push(request);
        yield* script(request);
      },
      respondToSessionPermission: async () => ({}),
      setSessionConfig: async () => ({ configOptions: [] }),
    } satisfies TestHandlers,
  };
}
