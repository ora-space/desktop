import type { ChatSessionClient } from "@ora/chat";
import type { PromptSessionEvent, PromptSessionRequest } from "@ora/contracts";

/** A chat client whose prompt stream is supplied by a deterministic test script. */
export interface ScriptedChatSession extends ChatSessionClient {
  readonly promptRequests: PromptSessionRequest[];
}

/**
 * Creates the smallest fake session boundary needed by interaction tests.
 *
 * History completes immediately, while each prompt is recorded before its scripted
 * events are yielded. Tests can therefore pause a response at an exact event boundary
 * and exercise the UI while the turn is genuinely streaming.
 */
export function createScriptedChatSession(
  script: (request: PromptSessionRequest) => AsyncIterable<PromptSessionEvent>,
): ScriptedChatSession {
  const promptRequests: PromptSessionRequest[] = [];

  return {
    promptRequests,
    load: async function* () {
      yield { type: "completed" as const };
    },
    prompt: async function* (request) {
      promptRequests.push(request);
      yield* script(request);
    },
    respondToPermission: async () => ({}),
    setConfig: async () => ({ configOptions: [] }),
  };
}
