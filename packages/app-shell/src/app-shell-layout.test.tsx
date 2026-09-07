import { act, fireEvent, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { createChatStore } from "@ora/chat";
import { createMockWorkflows } from "@ora/workflow-mock";
import {
  serializeWorkflowGraph,
  type WorkflowDefinitionEdge,
  type WorkflowDefinitionNode,
} from "@ora/workflow-runtime";
import { AppShell } from "./app-shell";
import { appI18n } from "./i18n/i18n-instance";
import {
  createTestClient,
  type TestHandlers,
} from "./test/contracts-transport";
import {
  createWorkspaceMemory,
  workspaceHandlers,
} from "./test/memory/workspaces";
import { createSessionMemory, sessionHandlers } from "./test/memory/sessions";
import {
  createAgentRuntimeMemory,
  agentRuntimeHandlers,
} from "./test/memory/agent-runtime";
import { createPluginMemory, pluginHandlers } from "./test/memory/plugins";
import { createAgentMemory, agentHandlers } from "./test/memory/agents";
import { createSkillMemory, skillHandlers } from "./test/memory/skills";
import { createSettingsMemory, settingsHandlers } from "./test/memory/settings";
import {
  createWorkflowMemory,
  workflowHandlers,
} from "./test/memory/workflows";
import { appEventHandlers } from "./test/memory/app-events";
import { createStubPlatform } from "./test/stub-platform";
import { useUiStore } from "./state/stores/ui-store";
import { useWorkflowEditorStore } from "./features/workflow-editor/workflow-editor-store";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return {
    ...createWorkspaceMemory(),
    ...createSessionMemory(),
    ...createAgentRuntimeMemory(),
    ...createPluginMemory(),
    ...createAgentMemory(),
    ...createSkillMemory(),
    ...createSettingsMemory(),
    ...createWorkflowMemory(),
  };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...workspaceHandlers(state),
    ...sessionHandlers(state),
    ...agentRuntimeHandlers(state),
    ...pluginHandlers(state),
    ...agentHandlers(state),
    ...skillHandlers(state),
    ...settingsHandlers(state),
    ...workflowHandlers(state),
    ...appEventHandlers(),
  };
}

/** Puts one named draft in the mock library so the editor can hydrate a title field. */
function seedNamedDraft(state: FixtureState, name: string): void {
  const workflow = createMockWorkflows("zh-CN")[0];
  if (workflow === undefined) {
    throw new Error("expected a demo workflow fixture");
  }
  const now = BigInt(Date.parse(workflow.updatedAt));
  state.workflows = [
    {
      workflow: {
        id: workflow.id,
        namespace: "local",
        name,
        publishedSnapshotId: null,
        createdAt: now,
        updatedAt: now,
      },
      draft: {
        id: `snap-${workflow.id}`,
        workflowId: workflow.id,
        version: "draft",
        graph: serializeWorkflowGraph({
          nodes: workflow.nodes as unknown as WorkflowDefinitionNode[],
          edges: workflow.edges as unknown as WorkflowDefinitionEdge[],
          viewport: workflow.viewport,
          description: workflow.description,
        }),
        createdAt: now,
        updatedAt: now,
      },
      published: [],
    },
  ];
}

beforeEach(async () => {
  window.localStorage.clear();
  useUiStore.setState({
    sidebarCollapsed: false,
    workflowEditorOpen: true,
  });
  useWorkflowEditorStore.setState({
    selectedWorkflowId: null,
    managerError: null,
    actions: null,
  });
  await act(() => appI18n.changeLanguage("zh-CN"));
  Object.defineProperty(HTMLElement.prototype, "clientWidth", {
    configurable: true,
    get() {
      return 1200;
    },
  });
  Object.defineProperty(HTMLElement.prototype, "clientHeight", {
    configurable: true,
    get() {
      return 800;
    },
  });
});

describe("AppShell sidebar collapse", () => {
  it("keeps in-memory workflow draft edits when the sidebar collapses", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    state.projects = [{ id: "p1", name: "Demo" }];
    seedNamedDraft(state, "代码审查工作流");
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);

    render(
      <AppShell
        client={client}
        chatStore={createChatStore(client.session)}
        platform={createStubPlatform()}
        user={{ name: "Eric", email: "eric@example.com" }}
      />,
    );

    // The editor hydrates several ticks after mount; under heavy parallel
    // suite load the findBy default of 1s gives up long before the test-level
    // timeout would, so this wait carries its own ceiling.
    const nameInput = await screen.findByLabelText(
      "工作流名称",
      {},
      { timeout: 10_000 },
    );
    fireEvent.change(nameInput, { target: { value: "折叠后仍在" } });
    expect(nameInput).toHaveValue("折叠后仍在");

    await user.click(screen.getByRole("button", { name: "收起侧边栏" }));

    expect(useUiStore.getState().sidebarCollapsed).toBe(true);
    expect(screen.getByLabelText("工作流名称")).toHaveValue("折叠后仍在");
  });
});
