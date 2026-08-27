import { waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { AgentStatus } from "@ora/contracts";
import {
  createMockClient,
  createMockClientState,
  type MockClientState,
} from "../../test/mock-client";
import { renderHookWithClient } from "../../test/hook-harness";
import { useAgentRuntimeStatus } from "./use-agent-runtime-status";
import { useAvailableAgents } from "./use-available-agents";

/** Every agent the seeded installation supplies, in the order its packages are listed. */
const INSTALLED = [
  "ora-space.opencode",
  "ora-space.nga",
  "ora-space.codeagentcli",
  "ora-space.claude",
  "ora-space.codex",
];

/** The installed set without OpenCode, which every "one agent is missing" case here drops. */
const WITHOUT_OPENCODE = INSTALLED.filter(
  (agentRef) => agentRef !== "ora-space.opencode",
);

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
 * Renders the hook against a settled detection status and returns what it offers.
 *
 * The status is awaited through the same query the hook reads, because the loading answer is the
 * whole catalog: asserting before it settles would pass for reasons the test is not about.
 */
async function offeredAgents(
  seed: (state: MockClientState) => void,
): Promise<string[]> {
  const state = createMockClientState();
  seed(state);
  const { result } = renderHookWithClient(
    () => ({
      offered: useAvailableAgents(),
      statuses: useAgentRuntimeStatus(),
    }),
    createMockClient(state),
  );
  await waitFor(() => expect(result.current.statuses.isSuccess).toBe(true));
  await waitFor(() => expect(result.current.offered.length).toBeGreaterThan(0));
  return result.current.offered.map((agent) => agent.agentRef);
}

describe("useAvailableAgents", () => {
  it("offers every installed agent the runtime reports reaching", async () => {
    expect(await offeredAgents(() => {})).toEqual(INSTALLED);
  });

  it("names each agent the way its installed package declares", async () => {
    const { result } = renderHookWithClient(
      () => ({
        offered: useAvailableAgents(),
        statuses: useAgentRuntimeStatus(),
      }),
      createMockClient(createMockClientState()),
    );
    await waitFor(() => expect(result.current.statuses.isSuccess).toBe(true));
    await waitFor(() =>
      expect(result.current.offered.length).toBeGreaterThan(0),
    );

    expect(result.current.offered[0]).toEqual({
      agentRef: "ora-space.opencode",
      label: "OpenCode",
      logo: null,
    });
  });

  it("offers an agent still completing its handshake", async () => {
    expect(await offeredAgents(reportOpenCode("starting"))).toEqual(INSTALLED);
  });

  it("withholds an agent nothing answered for", async () => {
    expect(await offeredAgents(reportOpenCode("unavailable"))).toEqual(
      WITHOUT_OPENCODE,
    );
  });

  it("withholds an agent the supervisor has given up on", async () => {
    expect(await offeredAgents(reportOpenCode("failing"))).toEqual(
      WITHOUT_OPENCODE,
    );
  });

  it("withholds an agent nothing supervises at all", async () => {
    expect(
      await offeredAgents((state) => {
        state.agentRuntimeStatuses = state.agentRuntimeStatuses.filter(
          (status) => status.agentRef !== "ora-space.opencode",
        );
      }),
    ).toEqual(WITHOUT_OPENCODE);
  });

  it("withholds an agent whose package is not installed here", async () => {
    expect(
      await offeredAgents((state) => {
        state.installedPlugins = state.installedPlugins.filter(
          (plugin) => plugin.name !== "ora-space.claude",
        );
      }),
    ).toEqual(INSTALLED.filter((agentRef) => agentRef !== "ora-space.claude"));
  });

  it("offers the whole installed catalog while the detection status is still loading", async () => {
    const { result } = renderHookWithClient(
      () => useAvailableAgents(),
      createMockClient(createMockClientState()),
    );

    await waitFor(() => expect(result.current.length).toBeGreaterThan(0));
    expect(result.current.map((agent) => agent.agentRef)).toEqual(INSTALLED);
  });
});
