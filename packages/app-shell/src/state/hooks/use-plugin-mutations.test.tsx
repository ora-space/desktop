import { act, waitFor } from "@testing-library/react";
import { useQuery } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import {
  createTestQueryClient,
  renderHookWithClient,
} from "../../test/hook-harness";
import { useAgentModelStore } from "../stores/agent-model-store";
import { queryKeys } from "./query-keys";
import { usePluginMutations } from "./use-plugin-mutations";

const AGENT_REF = "ora-space.opencode";
const PLUGIN_ID = `official/${AGENT_REF}`;
const TARGET = { type: "workspace" as const, workspaceId: "workspace-1" };

beforeEach(() => {
  useAgentModelStore.setState({ known: {} });
});

describe("usePluginMutations", () => {
  it("invalidates agent state and clears models after uninstall", async () => {
    const state = createMockClientState();
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
    const baseClient = createMockClient(state);
    const client = {
      ...baseClient,
      plugin: {
        ...baseClient.plugin,
        uninstall: async (
          ...args: Parameters<typeof baseClient.plugin.uninstall>
        ) => {
          const response = await baseClient.plugin.uninstall(...args);
          state.agentRuntimeStatuses = state.agentRuntimeStatuses.filter(
            (status) => status.agentRef !== AGENT_REF,
          );
          return response;
        },
      },
    };
    const queryClient = createTestQueryClient();
    const queryKey = queryKeys.warmSession(TARGET, AGENT_REF);
    const loadModels = vi.fn(async () => ({ catalog: "current" }));
    useAgentModelStore.getState().remember(AGENT_REF, state.configOptions);

    const { result } = renderHookWithClient(
      () => ({
        runtime: useQuery({
          queryKey: queryKeys.agentRuntimeStatus,
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
    expect(useAgentModelStore.getState().known[AGENT_REF]).toBeUndefined();
    expect(loadModels).toHaveBeenCalledOnce();
  });
});
