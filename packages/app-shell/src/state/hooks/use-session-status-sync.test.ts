import { describe, it, expect } from "vitest";
import { waitFor } from "@testing-library/react";
import { createChatStore, type AcpClient, type ChatStore } from "@ora/chat";
import type { acp, ContractsClient, Session } from "@ora/contracts";
import { createMockClient, createMockClientState, type MockClientState } from "../../test/mock-client";
import { renderHookWithClient } from "../../test/hook-harness";
import { useSessionStatusSync } from "./use-session-status-sync";
import { useSessions } from "./use-sessions";
import { queryKeys } from "./query-keys";

const SESSION: Session = {
  id: "s1",
  taskId: "t1",
  agentId: "codex",
  agentSessionId: "agent-1",
  status: "stopped",
};

/** An ACP client whose single prompt settles only when the test decides. */
class ControllableAcpClient implements AcpClient {
  private settle: ((response: acp.PromptResponse) => void) | null = null;
  private fail: ((error: Error) => void) | null = null;

  async newSession(): Promise<acp.NewSessionResponse> {
    return { sessionId: "agent-1" };
  }

  prompt(): Promise<acp.PromptResponse> {
    return new Promise((resolve, reject) => {
      this.settle = resolve;
      this.fail = reject;
    });
  }

  subscribe(): () => void {
    return () => undefined;
  }

  endTurn(stopReason: acp.StopReason): void {
    this.settle?.({ stopReason });
  }

  dropStream(message: string): void {
    this.fail?.(new Error(message));
  }
}

/** Seeds one session, mounts the sync hook, and returns the pieces tests drive. */
async function mountSync(): Promise<{
  state: MockClientState;
  client: ContractsClient;
  chatStore: ChatStore;
  acp: ControllableAcpClient;
  status: () => string;
}> {
  const state = createMockClientState();
  state.sessions.push({ ...SESSION });
  const client = createMockClient(state);
  const acp = new ControllableAcpClient();
  const chatStore = createChatStore(acp);

  const sessions = renderHookWithClient(() => useSessions(), client, undefined, chatStore);
  await waitFor(() => expect(sessions.result.current.isSuccess).toBe(true));
  renderHookWithClient(
    () => useSessionStatusSync(client, chatStore),
    client,
    sessions.queryClient,
    chatStore,
  );

  return {
    state,
    client,
    chatStore,
    acp,
    queryClient: sessions.queryClient,
    status: () => state.sessions[0]!.status,
  };
}

/** Starts a prompt without awaiting it, swallowing the expected rejection. */
function startPrompt(chatStore: ChatStore): Promise<void> {
  return chatStore
    .getState()
    .sendMessage({ oraSessionId: "s1", agentSessionId: "agent-1", text: "hi" })
    .catch(() => undefined);
}

describe("useSessionStatusSync", () => {
  it("marks the session running while a prompt is in flight", async () => {
    const { chatStore, acp, status } = await mountSync();

    const sending = startPrompt(chatStore);
    await waitFor(() => expect(status()).toBe("running"));

    acp.endTurn("end_turn");
    await sending;
  });

  it("returns the session to stopped once the turn ends", async () => {
    const { chatStore, acp, status } = await mountSync();

    const sending = startPrompt(chatStore);
    await waitFor(() => expect(status()).toBe("running"));
    acp.endTurn("end_turn");
    await sending;

    await waitFor(() => expect(status()).toBe("stopped"));
  });

  it.each(["max_tokens", "refusal", "cancelled"] as const)(
    "returns the session to stopped when the turn ends with %s",
    async (stopReason) => {
      const { chatStore, acp, status } = await mountSync();

      const sending = startPrompt(chatStore);
      await waitFor(() => expect(status()).toBe("running"));
      acp.endTurn(stopReason);
      await sending;

      await waitFor(() => expect(status()).toBe("stopped"));
    },
  );

  it("returns the session to stopped when the stream drops mid-turn", async () => {
    const { chatStore, acp, status } = await mountSync();

    const sending = startPrompt(chatStore);
    await waitFor(() => expect(status()).toBe("running"));
    acp.dropStream("stream dropped");
    await sending;

    await waitFor(() => expect(status()).toBe("stopped"));
  });

  it("leaves the session running while a stalled turn never settles", async () => {
    const { chatStore, status } = await mountSync();

    void startPrompt(chatStore);
    await waitFor(() => expect(status()).toBe("running"));

    // The agent never reported a stop, so running is the honest answer.
    await new Promise((resolve) => setTimeout(resolve, 20));
    expect(status()).toBe("running");
  });

  it("refreshes the cached session list so views observe the new status", async () => {
    const { chatStore, acp, queryClient } = await mountSync();
    const cachedStatus = () =>
      queryClient.getQueryData<Session[]>(queryKeys.sessions)?.[0]?.status;
    expect(cachedStatus()).toBe("stopped");

    const sending = startPrompt(chatStore);
    await waitFor(() => expect(cachedStatus()).toBe("running"));

    acp.endTurn("end_turn");
    await sending;
    await waitFor(() => expect(cachedStatus()).toBe("stopped"));
  });

  it("writes nothing when the session row no longer exists", async () => {
    const { state, chatStore, acp } = await mountSync();
    state.sessions.length = 0;

    const sending = startPrompt(chatStore);
    acp.endTurn("end_turn");
    await sending;

    expect(state.sessions).toEqual([]);
  });
});
