import { act, waitFor } from "@testing-library/react";
import { useQuery } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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
import {
  createTestQueryClient,
  renderHookWithClient,
} from "../../test/hook-harness";
import { usePluginOperationStore } from "../stores/plugin-operation-store";
import { agentRuntimeKeys } from "../data/agent-runtime";
import { usePluginMutations } from "./use-plugin-mutations";

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

const AGENT_REF = "ora-space.opencode";
const PLUGIN_ID = `official/${AGENT_REF}`;
const TARGET = { type: "workspace" as const, workspaceId: "workspace-1" };

beforeEach(() => {});

afterEach(() => {
  act(() => usePluginOperationStore.setState({ activities: {} }));
});

describe("usePluginMutations", () => {
  it("invalidates agent state and clears models after uninstall", async () => {
    const state = createFixtureState();
    state.installedPlugins = [
      {
        id: PLUGIN_ID,
        namespace: "official",
        name: AGENT_REF,
        displayName: "OpenCode",
        version: "1.0.0",
        description: "OpenCode agent",
        homepage: null,
        license: null,
        kind: "agent",
        agentDisplayName: "OpenCode",
        logo: null,
        installationValidity: { validity: "valid" },
        configuration: { state: "not_declared" },
        runtime: "running",
      },
    ];
    const baseClientHandlers: TestHandlers = createFixtureHandlers(state);
    const baseClient = createTestClient(baseClientHandlers);
    const client = createTestClient({
      ...baseClientHandlers,
      uninstallPlugin: async (
        ...args: Parameters<typeof baseClient.plugin.uninstall>
      ) => {
        const response = await baseClient.plugin.uninstall(...args);
        state.agentRuntimeStatuses = state.agentRuntimeStatuses.filter(
          (status) => status.agentRef !== AGENT_REF,
        );
        return response;
      },
    });
    const queryClient = createTestQueryClient();
    const queryKey = agentRuntimeKeys.agentModels(
      AGENT_REF,
      TARGET.workspaceId,
    );
    const loadModels = vi.fn(async () => ({ catalog: "current" }));

    const { result } = renderHookWithClient(
      () => ({
        runtime: useQuery({
          queryKey: agentRuntimeKeys.agentRuntimeStatus,
          queryFn: () =>
            client.agentRuntime
              .getStatus({})
              .then((response) => response.statuses),
        }),
        models: useQuery({
          queryKey,
          queryFn: loadModels,
          staleTime: Infinity,
        }),
        mutations: usePluginMutations(PLUGIN_ID, AGENT_REF),
      }),
      client,
      queryClient,
    );

    await waitFor(() => expect(result.current.runtime.isSuccess).toBe(true));
    await waitFor(() => expect(result.current.models.isSuccess).toBe(true));
    await act(async () => {
      await result.current.mutations.uninstall.mutateAsync("delete");
    });

    await waitFor(() =>
      expect(
        result.current.runtime.data?.some(
          (status) => status.agentRef === AGENT_REF,
        ),
      ).toBe(false),
    );
    expect(loadModels).toHaveBeenCalledOnce();
  });

  it("keeps uninstall pending across unmount and rejects a duplicate operation", async () => {
    const clientHandlers: TestHandlers =
      createFixtureHandlers(createFixtureState());
    const client = createTestClient(clientHandlers);
    let resolveUninstall:
      | ((
          response: Awaited<ReturnType<typeof client.plugin.uninstall>>,
        ) => void)
      | undefined;
    const uninstall = vi
      .spyOn(clientHandlers, "uninstallPlugin")
      .mockImplementation(
        () =>
          new Promise((resolve) => {
            resolveUninstall = resolve;
          }),
      );
    const stop = vi.spyOn(client.plugin, "stop");
    const queryClient = createTestQueryClient();
    const first = renderHookWithClient(
      () => usePluginMutations(PLUGIN_ID),
      client,
      queryClient,
    );

    act(() => first.result.current.uninstall.mutate("delete"));
    await waitFor(() => expect(uninstall).toHaveBeenCalledOnce());
    expect(first.result.current.uninstall.isPending).toBe(true);
    first.unmount();

    const second = renderHookWithClient(
      () => usePluginMutations(PLUGIN_ID),
      client,
      queryClient,
    );
    expect(second.result.current.uninstall.isPending).toBe(true);
    act(() => second.result.current.stop.mutate());
    act(() => second.result.current.uninstall.mutate("delete"));
    expect(uninstall).toHaveBeenCalledOnce();
    expect(stop).not.toHaveBeenCalled();

    await act(async () => {
      resolveUninstall?.({ pluginId: PLUGIN_ID });
    });
    await waitFor(() =>
      expect(second.result.current.uninstall.isPending).toBe(false),
    );
  });
});
