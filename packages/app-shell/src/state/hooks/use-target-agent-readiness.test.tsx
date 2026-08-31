import { act, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import type {
  AgentStatus,
  GetAgentRuntimeStatusResponse,
} from "@ora/contracts";
import {
  createMockClient,
  createMockClientState,
  type MockClientState,
} from "../../test/mock-client";
import { renderHookWithClient } from "../../test/hook-harness";
import { DEFAULT_SETTINGS, useSettingsStore } from "../stores/settings-store";
import { usePendingAgentStore } from "../stores/pending-agent-store";
import { useTargetAgentReadiness } from "./use-target-agent-readiness";

/** The project-only surface every case here resolves: no session, no task. */
const PROJECT_SELECTION = { projectId: "p1", taskId: null, sessionId: null };

/**
 * Points the shared default at the agent every case varies, so the hook resolves
 * through the same leg a fresh composer does and only detection status differs.
 */
beforeEach(() => {
  useSettingsStore.setState({
    settings: { ...DEFAULT_SETTINGS, agentCli: "ora-space.opencode" },
  });
  usePendingAgentStore.setState({ selections: {}, switches: {} });
});

/** Replaces what the runtime reports about one agent, leaving the rest detected. */
function reportOpenCode(status: AgentStatus) {
  return (state: MockClientState) => {
    const entry = state.agentRuntimeStatuses.find(
      (candidate) => candidate.agentRef === "ora-space.opencode",
    );
    entry!.status = status;
  };
}

/**
 * Renders the hook and waits for detection to settle, because `unknown` is also
 * the loading answer: asserting before the query lands would pass for a reason
 * the test is not about.
 */
async function readiness(
  seed: (state: MockClientState) => void = () => {},
): Promise<ReturnType<typeof useTargetAgentReadiness>> {
  const state = createMockClientState();
  seed(state);
  const { result } = renderHookWithClient(
    () => useTargetAgentReadiness(PROJECT_SELECTION),
    createMockClient(state),
  );
  await waitFor(() => expect(result.current).not.toBe("unknown"));
  return result.current;
}

describe("useTargetAgentReadiness", () => {
  it("reports ready when the runtime detects the resolved agent", async () => {
    expect(await readiness()).toBe("ready");
  });

  it("blocks an agent still completing its handshake", async () => {
    // The picker offers `starting` agents, but a send into one is exactly the
    // failure this gate exists to prevent, so it must read as blocked here.
    expect(await readiness(reportOpenCode("starting"))).toBe("blocked");
  });

  it("blocks an agent nothing answered for", async () => {
    expect(await readiness(reportOpenCode("unavailable"))).toBe("blocked");
  });

  it("blocks an agent the supervisor has given up on", async () => {
    expect(await readiness(reportOpenCode("failing"))).toBe("blocked");
  });

  it("blocks an agent absent from the detection report", async () => {
    expect(
      await readiness((state) => {
        state.agentRuntimeStatuses = state.agentRuntimeStatuses.filter(
          (status) => status.agentRef !== "ora-space.opencode",
        );
      }),
    ).toBe("blocked");
  });

  it("blocks a surface that never resolved an agent", async () => {
    useSettingsStore.setState({
      settings: { ...DEFAULT_SETTINGS, agentCli: null },
    });
    expect(await readiness()).toBe("blocked");
  });

  it("stays unknown while detection never answers", async () => {
    const state = createMockClientState();
    const client = createMockClient(state);
    const stalled = {
      ...client,
      agentRuntime: {
        ...client.agentRuntime,
        getStatus: () => new Promise<GetAgentRuntimeStatusResponse>(() => {}),
      },
    };
    const { result } = renderHookWithClient(
      () => useTargetAgentReadiness(PROJECT_SELECTION),
      stalled,
    );

    // The pending window must not read as blocked, or every send would freeze
    // until the status query lands; flush the render pass and confirm it held.
    await act(async () => {});
    expect(result.current).toBe("unknown");
  });
});
