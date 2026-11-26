import type { ComponentProps } from "react";
import { act, render, screen, waitFor } from "@testing-library/react";
import { createChatStore, type ChatTurn } from "@ora/chat";
import type { ContractsClient } from "@ora/contracts";
import { TooltipProvider } from "@ora/ui";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AppI18nProvider } from "../../i18n/i18n";
import { PlatformProvider } from "../../platform";
import { useSessionSetupStore } from "../chat/session-setup";
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
import { usePendingAgentStore } from "../../state/stores/pending-agent-store";
import { useAgentModelPreferenceStore } from "../../state/stores/agent-model-preference-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useDraftSessionsStore } from "../../state/stores/draft-sessions-store";
import { useComposerInputStore } from "../../state/stores/composer-input-store";
import { useUiStore } from "../../state/stores/ui-store";
import {
  DEFAULT_SETTINGS,
  useSettingsStore,
} from "../../state/stores/settings-store";
import { useWorkflowEditorStore } from "../workflow-editor/workflow-editor-store";
import { AGENT_REF } from "../../test/agent-identity";
import { WorkspaceView } from "./workspace-view";

const reviewRenders = vi.hoisted(() => ({ count: 0 }));

vi.mock("./workspace-review-layout", async () => {
  const React = await import("react");
  const actual = await vi.importActual<
    typeof import("./workspace-review-layout")
  >("./workspace-review-layout");
  return {
    ...actual,
    WorkspaceReviewLayout: (
      props: ComponentProps<typeof actual.WorkspaceReviewLayout>,
    ) => {
      reviewRenders.count += 1;
      return React.createElement(actual.WorkspaceReviewLayout, props);
    },
  };
});

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
  };
}

function streamedTurn(): ChatTurn {
  return {
    id: "turn-stream",
    userMessage: {
      kind: "message",
      id: "turn-stream-user",
      role: "user",
      content: "ask for a token",
      createdAt: 1,
    },
    items: [
      {
        kind: "message",
        id: "turn-stream-assistant",
        role: "assistant",
        content: "stream token alpha",
        createdAt: 2,
      },
    ],
    status: "completed",
    stopReason: null,
    error: null,
    createdAt: 1,
  };
}

beforeEach(() => {
  reviewRenders.count = 0;
  useWorkspaceSelectionStore.getState().clearSelection();
  useDraftSessionsStore.getState().clear();
  useComposerInputStore.getState().reset();
  useSessionSetupStore.setState({ setups: {} });
  useUiStore.setState({ workflowEditorOpen: false });
  useWorkflowEditorStore.setState({
    selectedWorkflowId: null,
    managerError: null,
    actions: null,
    renderRecovery: { status: "ready", revision: 0 },
  });
  usePendingAgentStore.setState({ selections: {}, switches: {}, models: {} });
  useAgentModelPreferenceStore.setState({ models: {} });
  useSettingsStore.setState({
    settings: { ...DEFAULT_SETTINGS, agentCli: AGENT_REF.opencode },
  });
});

describe("WorkspaceView streaming renders", () => {
  it("keeps the review layout mounted while a token updates the chat", async () => {
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Ora" }];
    state.tasks = [
      {
        id: "t1",
        projectId: "p1",
        workspaceId: "workspace-t1",
        title: "Stream",
      },
    ];
    state.sessions = [
      {
        id: "s1",
        workspaceId: "workspace-t1",
        agentRef: AGENT_REF.opencode,
        status: "running",
        title: null,
        historyState: { type: "writable" },
        mcpSelection: { mode: "automatic" },
      },
    ];
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client: ContractsClient = createTestClient(clientHandlers);
    clientHandlers.loadSession = vi.fn(async function* () {
      yield { type: "completed" as const };
    });
    const chatStore = createChatStore(client.session);
    const Wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      chatStore,
    );
    useWorkspaceSelectionStore.getState().selectSession("s1", "t1", "p1");

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

    await waitFor(() =>
      expect(chatStore.getState().conversations.s1?.isLoaded).toBe(true),
    );
    const rendersAfterLoad = reviewRenders.count;
    expect(rendersAfterLoad).toBeGreaterThan(0);

    act(() => {
      chatStore.setState((current) => {
        const conversation = current.conversations.s1;
        if (conversation === undefined) return current;
        return {
          conversations: {
            ...current.conversations,
            s1: { ...conversation, turns: [streamedTurn()] },
          },
        };
      });
    });

    expect(screen.getByText("stream token alpha")).toBeInTheDocument();
    expect(reviewRenders.count).toBe(rendersAfterLoad);
  });
});
