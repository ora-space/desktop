import { act, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAgentDiscovery } from "./use-agent-discovery";
import { useTargetAgentCli } from "./use-target-agent-cli";
import { useTargetAgentReadiness } from "./use-target-agent-readiness";
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

it("upgrades a v1 preference to the installed identity used by model discovery", async () => {
  localStorage.setItem(
    "ora.settings.v1",
    JSON.stringify({
      version: 1,
      state: {
        settings: { ...DEFAULT_SETTINGS, agentCli: "ora-space.opencode" },
      },
    }),
  );
  await act(async () => {
    await useSettingsStore.persist.rehydrate();
  });
  const state = createFixtureState();
  const handlers = createFixtureHandlers(state);
  const discover = vi.fn(handlers.discoverAgent!);
  const { result } = renderHookWithClient(
    () => {
      useDefaultAgentAdoption();
      const selected = useTargetAgentCli({
        projectId: null,
        taskId: null,
        sessionId: null,
      });
      return {
        selected,
        readiness: useTargetAgentReadiness({
          projectId: null,
          taskId: null,
          sessionId: null,
        }),
        models: useAgentDiscovery(selected, "workspace-upgrade", "models"),
      };
    },
    createTestClient({
      ...handlers,
      discoverAgent: discover,
      listSessions: async () => ({ sessions: [] }),
    }),
  );
  await waitFor(() =>
    expect(result.current.models.models.length).toBeGreaterThan(0),
  );
  expect(result.current.selected).toBe(AGENT_REF.opencode);
  expect(result.current.readiness).toBe("ready");
  expect(discover.mock.calls[0]?.[0]).toEqual({
    agentRef: AGENT_REF.opencode,
    workspaceId: "workspace-upgrade",
    includeCommands: false,
  });
  expect(
    JSON.parse(localStorage.getItem("ora.settings.v1")!).state.settings
      .agentCli,
  ).toBe(AGENT_REF.opencode);
});

it("preserves an ambiguous legacy identity instead of choosing a source", async () => {
  useSettingsStore.setState({
    settings: { ...DEFAULT_SETTINGS, agentCli: "ora-space.opencode" },
  });
  await renderAdoption((state) => {
    state.installedPlugins.push({
      ...state.installedPlugins[0]!,
      id: "thirdparty/ora-space.opencode",
    });
  });
  await expectAdopted("ora-space.opencode");
});

it("preserves an absent legacy identity instead of adopting another agent", async () => {
  useSettingsStore.setState({
    settings: { ...DEFAULT_SETTINGS, agentCli: "missing.agent" },
  });
  await renderAdoption();
  await expectAdopted("missing.agent");
});

it("renders a malformed persisted agent preference without overwriting it", async () => {
  for (const agentCli of [42, { broken: true }, ["ora-space.opencode"]]) {
    localStorage.setItem(
      "ora.settings.v1",
      JSON.stringify({
        version: 2,
        state: { settings: { ...DEFAULT_SETTINGS, agentCli } },
      }),
    );
    await act(async () => {
      await useSettingsStore.persist.rehydrate();
    });
    await renderAdoption();
    expect(useSettingsStore.getState().settings.agentCli).toEqual(agentCli);
  }
});
