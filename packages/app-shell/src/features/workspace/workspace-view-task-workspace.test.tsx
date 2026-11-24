import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { createChatStore } from "@ora/chat";
import type { ContractsClient, StartSessionRequest } from "@ora/contracts";
import { TooltipProvider } from "@ora/ui";
import { PlatformProvider } from "../../platform";
import { beforeEach, describe, expect, it } from "vitest";
import "../../i18n/i18n-instance";
import { workspaceKeys } from "../../state/data/workspace";
import { AppI18nProvider } from "../../i18n/i18n";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createWorkspaceMemory,
  workspaceHandlers,
} from "../../test/memory/workspaces";
import {
  createSessionMemory,
  sessionHandlers,
} from "../../test/memory/sessions";
import {
  createAgentRuntimeMemory,
  agentRuntimeHandlers,
} from "../../test/memory/agent-runtime";
import { createPluginMemory, pluginHandlers } from "../../test/memory/plugins";
import { createAgentMemory, agentHandlers } from "../../test/memory/agents";
import { createSkillMemory, skillHandlers } from "../../test/memory/skills";
import {
  createWorkflowMemory,
  workflowHandlers,
} from "../../test/memory/workflows";
import { readyEffectHandlers } from "../../test/memory/effects";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useDraftSessionsStore } from "../../state/stores/draft-sessions-store";
import { useComposerInputStore } from "../../state/stores/composer-input-store";
import { useUiStore } from "../../state/stores/ui-store";
import { startSessionDraft } from "../../state/session-drafts";
import {
  DEFAULT_SETTINGS,
  useSettingsStore,
} from "../../state/stores/settings-store";
import { WorkspaceView } from "./workspace-view";
import { useWorkflowEditorStore } from "../workflow-editor/workflow-editor-store";
import { AGENT_REF } from "../../test/agent-identity";

/**
 * Regression coverage for the reported
 * "developed in the worktree, changes landed in the main checkout" scenario.
 *
 * State: a task exists on the backend (its isolated workspace is visible), the
 * user is composing in a draft scoped to that task — but the tasks list cache
 * does not contain the task (a delayed or failed tasks query before the
 * `list_tasks` refetch landing, or any refetch that has not answered yet).
 * The first send must not silently fall back to the project's main workspace.
 */
function createFixtureState() {
  return {
    ...createWorkspaceMemory(),
    ...createSessionMemory(),
    ...createAgentRuntimeMemory(),
    ...createPluginMemory(),
    ...createAgentMemory(),
    ...createSkillMemory(),
    ...createWorkflowMemory(),
  };
}

type FixtureState = ReturnType<typeof createFixtureState>;

function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...workspaceHandlers(state),
    ...sessionHandlers(state),
    ...agentRuntimeHandlers(state),
    ...pluginHandlers(state),
    ...agentHandlers(state),
    ...skillHandlers(state),
    ...workflowHandlers(state),
    ...readyEffectHandlers(),
  };
}

beforeEach(() => {
  useWorkspaceSelectionStore.getState().clearSelection();
  useDraftSessionsStore.getState().clear();
  useComposerInputStore.getState().reset();
  useUiStore.setState({ workflowEditorOpen: false });
  useWorkflowEditorStore.setState({
    selectedWorkflowId: null,
    managerError: null,
    actions: null,
    renderRecovery: { status: "ready", revision: 0 },
  });
  useSettingsStore.setState({
    settings: { ...DEFAULT_SETTINGS, agentCli: AGENT_REF.opencode },
  });
});

describe("task-scoped first send without the task in the tasks cache", () => {
  it("rejects a missing task visibly and retries on its isolated workspace after loading", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    // The task row exists on the backend with its isolated workspace...
    state.workspaces.push({
      id: "workspace-t1",
      projectId: "p1",
      kind: "isolated",
      lifecycle: "active",
      directory: { kind: "present", git: "repository" },
    });
    // ...but the tasks list the client loaded does not contain it yet.
    const baseHandlers = createFixtureHandlers(state);
    const started: StartSessionRequest[] = [];
    const client: ContractsClient = createTestClient({
      ...baseHandlers,
      startSession: async (request, options) => {
        started.push(request);
        return baseHandlers.startSession!(request, options);
      },
    });
    const chatStore = createChatStore(client.session);
    const queryClient = createTestQueryClient();
    const Wrapper = createHookWrapper(client, queryClient, chatStore);

    // A restored task draft can be selected before the tasks query resolves.
    startSessionDraft({ projectId: "p1", taskId: "t1" });

    render(
      <Wrapper>
        <AppI18nProvider>
          <PlatformProvider adapter={createStubPlatform()}>
            <TooltipProvider>
              <WorkspaceView userName="Eric" />
            </TooltipProvider>
          </PlatformProvider>
        </AppI18nProvider>
      </Wrapper>,
    );

    const composer = await screen.findByRole("textbox");
    await waitFor(() => expect(composer).toBeEnabled());
    await user.type(composer, "develop in shard4");
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );

    await screen.findAllByText(
      "The selected task is not loaded. Please wait and try again.",
    );
    expect(started).toEqual([]);
    expect(useWorkspaceSelectionStore.getState().selection.taskId).toBe("t1");
    await waitFor(() =>
      expect(screen.getByRole("textbox")).toHaveTextContent(
        "develop in shard4",
      ),
    );

    await act(async () => {
      state.tasks = [
        {
          id: "t1",
          projectId: "p1",
          workspaceId: "workspace-t1",
          title: "Shard 4",
        },
      ];
      queryClient.setQueryData(workspaceKeys.tasks, state.tasks);
    });
    await user.click(
      screen.getByRole("button", { name: /发送消息|Send message/ }),
    );
    await waitFor(() => expect(started.length).toBe(1));
    // The assertion that matters: the send must not ride the main workspace.
    expect(started[0]!.workspaceId).toBe("workspace-t1");
  });
});
