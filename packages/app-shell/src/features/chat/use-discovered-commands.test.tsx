import { act, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { createTestClient } from "../../test/contracts-transport";
import {
  agentRuntimeHandlers,
  createAgentRuntimeMemory,
} from "../../test/memory/agent-runtime";
import "../../i18n/i18n-instance";
import { renderHookWithClient } from "../../test/hook-harness";
import { AGENT_REF } from "../../test/agent-identity";
import { invalidateAgentAvailability } from "../../state/data/agent-runtime";
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import { useDiscoveredCommands } from "./use-discovered-commands";

/** An attached session on OpenCode that has reported nothing, in a present folder. */
const UNREPORTED = {
  sessionId: "s1",
  hasReport: false,
  isLoadingHistory: false,
  agentRef: AGENT_REF.opencode,
  workspaceId: "w1",
  directoryMissing: false,
};

/** Renders the hook against agents that each report one command, and counts discoveries. */
function renderDiscovered(
  surface: () => Parameters<typeof useDiscoveredCommands>[0],
) {
  const state = createAgentRuntimeMemory();
  state.discoveredCommandsByCli = {
    [AGENT_REF.opencode]: [{ name: "open", description: "OpenCode's" }],
    [AGENT_REF.claude]: [{ name: "claude", description: "Claude's" }],
  };
  const client = createTestClient(agentRuntimeHandlers(state));
  const discover = vi.spyOn(client.agentRuntime, "discover");
  const { result, rerender, queryClient } = renderHookWithClient(
    () => useDiscoveredCommands(surface()),
    client,
  );
  return { result, rerender, discover, state, queryClient };
}

/** Lets a discovery that must not start have the chance to. */
async function settle() {
  await act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 20));
  });
}

beforeEach(() => {
  usePendingAgentStore.setState({ selections: {}, switches: {} });
});

describe("discovered commands", () => {
  it("stand in for a report the session has not made", async () => {
    const { result, discover } = renderDiscovered(() => UNREPORTED);

    expect(result.current?.commands).toBeNull();
    expect(result.current?.isLoading).toBe(true);
    await waitFor(() =>
      expect(result.current).toEqual({
        agentRef: AGENT_REF.opencode,
        commands: [{ name: "open", description: "OpenCode's" }],
        isLoading: false,
      }),
    );
    expect(discover).toHaveBeenCalledTimes(1);
    expect(discover.mock.calls[0]?.[0]).toEqual({
      agentRef: AGENT_REF.opencode,
      workspaceId: "w1",
      includeCommands: true,
    });
  });

  it("stay unknown when the agent reported none to discovery", async () => {
    const { result, state } = renderDiscovered(() => UNREPORTED);
    state.discoveredCommandsByCli = {};

    await waitFor(() => expect(result.current?.isLoading).toBe(false));
    expect(result.current?.commands).toBeNull();
  });

  it("keep an empty list apart from an unknown one", async () => {
    const { result, state } = renderDiscovered(() => UNREPORTED);
    state.discoveredCommandsByCli = { [AGENT_REF.opencode]: [] };

    await waitFor(() => expect(result.current?.commands).toEqual([]));
  });

  it.each([
    ["the session has reported", { hasReport: true }],
    ["the report may still be replayed", { isLoadingHistory: true }],
    ["no agent is chosen", { agentRef: null }],
  ])("are neither shown nor asked for while %s", async (_case, change) => {
    const { result, discover } = renderDiscovered(() => ({
      ...UNREPORTED,
      ...change,
    }));

    await settle();
    expect(result.current).toBeUndefined();
    expect(discover).not.toHaveBeenCalled();
  });

  it("do not ask an agent about a folder that is gone", async () => {
    const { result, discover } = renderDiscovered(() => ({
      ...UNREPORTED,
      directoryMissing: true,
    }));

    await settle();
    expect(result.current).toEqual({
      agentRef: AGENT_REF.opencode,
      commands: null,
      isLoading: false,
    });
    expect(discover).not.toHaveBeenCalled();
  });

  it("describe the incoming agent while a move is pending, whatever was reported", async () => {
    let agentRef: string = AGENT_REF.opencode;
    const { result } = renderDiscovered(() => ({
      ...UNREPORTED,
      hasReport: true,
      agentRef,
    }));
    expect(result.current).toBeUndefined();

    act(() => {
      agentRef = AGENT_REF.claude;
      usePendingAgentStore.getState().setPendingSwitch("s1", AGENT_REF.claude);
    });
    await waitFor(() =>
      expect(result.current).toEqual({
        agentRef: AGENT_REF.claude,
        commands: [{ name: "claude", description: "Claude's" }],
        isLoading: false,
      }),
    );

    act(() => {
      agentRef = AGENT_REF.opencode;
      usePendingAgentStore.getState().clearPendingSwitch("s1");
    });
    expect(result.current).toBeUndefined();
  });

  it("are withheld once their agent cannot be reached", async () => {
    const { result, state, queryClient } = renderDiscovered(() => UNREPORTED);
    await waitFor(() => expect(result.current?.commands).toHaveLength(1));

    state.agentRuntimeStatuses = state.agentRuntimeStatuses.map((status) => ({
      ...status,
      status: "unavailable",
    }));
    await act(() => invalidateAgentAvailability(queryClient));
    await waitFor(() => expect(result.current?.commands).toBeNull());
  });
});
