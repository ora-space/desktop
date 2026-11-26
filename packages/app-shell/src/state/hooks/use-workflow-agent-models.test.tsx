import { waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import "../../i18n/i18n-instance";
import { AGENT_REF } from "../../test/agent-identity";
import { createTestClient } from "../../test/contracts-transport";
import {
  createTestQueryClient,
  renderHookWithClient,
} from "../../test/hook-harness";
import {
  agentRuntimeHandlers,
  createAgentRuntimeMemory,
} from "../../test/memory/agent-runtime";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import {
  createWorkspaceMemory,
  workspaceHandlers,
} from "../../test/memory/workspaces";
import {
  invalidateAgentCommands,
  invalidateAgentModels,
} from "../data/agent-runtime";
import { useWorkspaceSelectionStore } from "../stores/workspace-selection-store";
import { useAgentDiscovery } from "./use-agent-discovery";
import { useWorkflowAgentModels } from "./use-workflow-agent-models";

/** Mounts the chat's discovery and the editor's catalog on one cache, in either order. */
function surfaces() {
  const state = {
    ...createWorkspaceMemory(),
    ...createAgentRuntimeMemory(),
    ...createPluginMemory(),
  };
  state.projects = [{ id: "p1", name: "Ora" }];
  state.discoveredCommandsByCli = {
    [AGENT_REF.opencode]: [{ name: "review", description: "Review" }],
  };
  useWorkspaceSelectionStore.getState().selectProject("p1");
  const client = createTestClient({
    ...workspaceHandlers(state),
    ...agentRuntimeHandlers(state),
    ...pluginHandlers(state),
  });
  const discover = vi.spyOn(client.agentRuntime, "discover");
  const queryClient = createTestQueryClient();
  // The app's default, under which an answer is reused rather than refreshed on mount.
  queryClient.setDefaultOptions({ queries: { staleTime: 30_000 } });
  return {
    queryClient,
    /** Whether each OpenCode discovery so far asked for commands. */
    asked: () =>
      discover.mock.calls
        .map(([request]) => request)
        .filter(({ agentRef }) => agentRef === AGENT_REF.opencode)
        .map(({ includeCommands }) => includeCommands),
    editor: async () => {
      const { result, unmount } = renderHookWithClient(
        useWorkflowAgentModels,
        client,
        queryClient,
      );
      await waitFor(() => {
        expect(
          result.current.modelsByCli.get(AGENT_REF.opencode)?.length,
        ).toBeGreaterThan(0);
        expect(result.current.isLoading).toBe(false);
      });
      unmount();
    },
    chat: async () => {
      const { result, unmount } = renderHookWithClient(
        () => useAgentDiscovery(AGENT_REF.opencode, "workspace-p1"),
        client,
        queryClient,
      );
      await waitFor(() =>
        expect(result.current.commands).toEqual([
          { name: "review", description: "Review" },
        ]),
      );
      unmount();
    },
  };
}

describe("the editor's model catalog and a chat's discovery", () => {
  it("never lets a models-only answer hide the commands", async () => {
    const { editor, chat, asked } = surfaces();

    await editor();
    await chat();

    expect(asked()).toEqual([false, true]);
  });

  it("answers the editor from a chat's discovery that is still good", async () => {
    const { editor, chat, asked } = surfaces();

    await chat();
    await editor();

    expect(asked()).toEqual([true]);
  });

  it.each(["editor", "chat"] as const)(
    "does not ask the editor's agents again when Skills change after the %s asked",
    async (first) => {
      const { editor, chat, asked, queryClient } = surfaces();

      await (first === "editor" ? editor() : chat());
      await invalidateAgentCommands(queryClient, AGENT_REF.opencode);
      await editor();
      expect(asked()).toEqual([first === "chat"]);

      await chat();
      expect(asked()).toEqual([first === "chat", true]);
    },
  );

  it("asks again when the chat's discovery was outdated", async () => {
    const { editor, chat, asked, queryClient } = surfaces();

    await chat();
    await invalidateAgentModels(queryClient, AGENT_REF.opencode);
    await editor();

    expect(asked()).toEqual([true, false]);
  });
});
