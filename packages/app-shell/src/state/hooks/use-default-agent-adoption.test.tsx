import { act, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import type {
  AgentStatus,
  GetAgentRuntimeStatusResponse,
} from "@ora/contracts";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createAgentRuntimeMemory,
  agentRuntimeHandlers,
} from "../../test/memory/agent-runtime";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import "../../i18n/i18n-instance";
import { renderHookWithClient } from "../../test/hook-harness";
import { agentRuntimeKeys } from "../data/agent-runtime";
import { DEFAULT_SETTINGS, useSettingsStore } from "../stores/settings-store";
import { useAgentRuntimeStatus } from "./use-agent-runtime-status";
import { useDefaultAgentAdoption } from "./use-default-agent-adoption";
import { AGENT_REF } from "../../test/agent-identity";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return { ...createAgentRuntimeMemory(), ...createPluginMemory() };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...agentRuntimeHandlers(state),
    ...pluginHandlers(state),
  };
}

/** The installation this file describes: one that has never chosen an agent. */
beforeEach(() => {
  useSettingsStore.setState({ settings: { ...DEFAULT_SETTINGS } });
});

/** Replaces what the runtime reports about one agent, leaving the rest detected. */
function reportOpenCode(status: AgentStatus) {
  return (state: FixtureState) => {
    const entry = state.agentRuntimeStatuses.find(
      (candidate) => candidate.agentRef === AGENT_REF.opencode,
    );
    entry!.status = status;
  };
}

/**
 * Mounts the adoption hook and resolves once detection has answered.
 *
 * Every case here turns on what the runtime reports, so asserting before that query lands would
 * be asserting against the loading answer instead of the behavior under test.
 */
async function renderAdoption(seed: (state: FixtureState) => void = () => {}) {
  const state = createFixtureState();
  seed(state);
  const { result, queryClient } = renderHookWithClient(
    () => {
      useDefaultAgentAdoption();
      return useAgentRuntimeStatus();
    },
    createTestClient(createFixtureHandlers(state)),
  );
  await waitFor(() => expect(result.current.isSuccess).toBe(true));
  return { state, queryClient };
}

/** Re-reports the seeded statuses after a case has mutated them. */
async function refreshDetection(
  queryClient: Awaited<ReturnType<typeof renderAdoption>>["queryClient"],
) {
  await act(() =>
    queryClient.invalidateQueries({
      queryKey: agentRuntimeKeys.agentRuntimeStatus,
    }),
  );
}

/** Waits for the stored preferences to settle on exactly this agent and nothing else. */
async function expectAdopted(agentRef: string | null) {
  await waitFor(() =>
    expect(useSettingsStore.getState().settings).toEqual({
      ...DEFAULT_SETTINGS,
      agentCli: agentRef,
    }),
  );
}

describe("useDefaultAgentAdoption", () => {
  it("adopts the first agent the runtime reports reaching", async () => {
    await renderAdoption();

    await expectAdopted(AGENT_REF.opencode);
  });

  it("passes over an agent nothing reaches", async () => {
    await renderAdoption(reportOpenCode("unavailable"));

    await expectAdopted(AGENT_REF.nga);
  });

  it("leaves a stored choice alone", async () => {
    useSettingsStore.setState({
      settings: { ...DEFAULT_SETTINGS, agentCli: AGENT_REF.claude },
    });

    await renderAdoption();

    await expectAdopted(AGENT_REF.claude);
  });

  /**
   * The whole installed catalog is the loading answer of the available-agent list, including
   * agents nothing supervises. Adopting from it would store a default no session could be opened
   * on, so the pending window must produce no preference at all.
   */
  it("adopts nothing while detection has not answered", async () => {
    const state = createFixtureState();
    const stalled = createTestClient({
      ...createFixtureHandlers(state),
      getAgentRuntimeStatus: () =>
        new Promise<GetAgentRuntimeStatusResponse>(() => {}),
    });
    renderHookWithClient(() => useDefaultAgentAdoption(), stalled);

    // Flush the render pass the installed-plugin query schedules, which is the one that would
    // offer the catalog if the guard were missing.
    await act(async () => {});
    expect(useSettingsStore.getState().settings.agentCli).toBeNull();
  });

  /**
   * Adoption is an initialization, not a fallback that keeps re-deciding: an agent arriving later
   * is exactly the plugin install this must not silently follow.
   */
  it("keeps what it adopted when another agent becomes available", async () => {
    const { state, queryClient } = await renderAdoption(
      reportOpenCode("unavailable"),
    );
    await expectAdopted(AGENT_REF.nga);

    reportOpenCode("ready")(state);
    await refreshDetection(queryClient);

    await expectAdopted(AGENT_REF.nga);
  });

  /**
   * The ordinary first run: the shell paints while agent processes are still completing their
   * handshake, so there is nothing to adopt yet and the first one to answer becomes the default.
   */
  it("waits for an installation that reaches no agent yet", async () => {
    const { state, queryClient } = await renderAdoption((seeded) => {
      seeded.agentRuntimeStatuses = [];
    });
    expect(useSettingsStore.getState().settings.agentCli).toBeNull();

    state.agentRuntimeStatuses = [
      { agentRef: AGENT_REF.claude, status: "ready" },
    ];
    await refreshDetection(queryClient);

    await expectAdopted(AGENT_REF.claude);
  });
});
