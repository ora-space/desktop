import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
  type RenderResult,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import { createChatStore } from "@ora/chat";
import { PlatformProvider } from "../../platform";
import {
  createMockWorkflow,
  createMockWorkflowVersions,
  createMockWorkflows,
  WORKFLOW_ITERATION_NODE_HEIGHT,
  WORKFLOW_ITERATION_NODE_WIDTH,
  WORKFLOW_NODE_INITIAL_HEIGHT,
  WORKFLOW_NODE_WIDTH,
  type DemoWorkflow,
} from "@ora/workflow-mock";
import {
  parseWorkflowGraph,
  serializeWorkflowGraph,
  type WorkflowDefinitionEdge,
  type WorkflowDefinitionNode,
} from "@ora/workflow-runtime";
import { TooltipProvider, toast } from "@ora/ui";
import { appI18n } from "../../i18n/i18n-instance";
import { AppI18nProvider } from "../../i18n/i18n";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createTestClient,
  type TestHandlers,
} from "../../test/contracts-transport";
import {
  createWorkspaceMemory,
  workspaceHandlers,
} from "../../test/memory/workspaces";
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
import { renderHookWithClient } from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import { useUiStore } from "../../state/stores/ui-store";
import {
  useCreateWorkflow,
  useDeleteWorkflow,
} from "../../state/data/workflows";
import { WorkflowEditor } from "./workflow-editor";
import { WorkflowEditorList } from "./workflow-editor-list";
import { useWorkflowEditorStore } from "./workflow-editor-store";
import { AGENT_REF } from "../../test/agent-identity";

/** State for this test surface; no unrelated domain fixtures are initialized. */
function createFixtureState() {
  return {
    ...createWorkspaceMemory(),
    ...createAgentRuntimeMemory(),
    ...createPluginMemory(),
    ...createAgentMemory(),
    ...createSkillMemory(),
    ...createWorkflowMemory(),
  };
}

type FixtureState = ReturnType<typeof createFixtureState>;

/** Explicit domain composition for the behaviors exercised by this test file. */
function createFixtureHandlers(state: FixtureState): TestHandlers {
  return {
    ...workspaceHandlers(state),
    ...agentRuntimeHandlers(state),
    ...pluginHandlers(state),
    ...agentHandlers(state),
    ...skillHandlers(state),
    ...workflowHandlers(state),
  };
}

/** Seeds the mock client with the demo workflows and their published versions. */
function seedDemoWorkflows(state: FixtureState): void {
  const locale =
    appI18n.resolvedLanguage === "en-US"
      ? ("en-US" as const)
      : ("zh-CN" as const);
  const demo = createMockWorkflows(locale);
  const versionsByWorkflow = createMockWorkflowVersions(demo);
  // Match the mock editor's default selection: open the code-review showcase first.
  demo.sort((a, b) =>
    a.id === "code-review" ? -1 : b.id === "code-review" ? 1 : 0,
  );
  state.workflows = demo.map((workflow) => {
    const now = BigInt(Date.parse(workflow.updatedAt));
    const record = {
      workflow: {
        id: workflow.id,
        namespace: "local",
        name: workflow.name,
        publishedSnapshotId: null as string | null,
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
          annotations: workflow.annotations ?? [],
          globalVariables: workflow.globalVariables ?? [],
          description: workflow.description,
        }),
        createdAt: now,
        updatedAt: now,
      },
      published: [] as {
        id: string;
        workflowId: string;
        version: string;
        graph: string;
        createdAt: bigint;
        updatedAt: bigint | null;
      }[],
    };
    (versionsByWorkflow[workflow.id] ?? []).forEach((version, index) => {
      record.published.push({
        id: `pub-${workflow.id}-${index}`,
        workflowId: workflow.id,
        version: version.version,
        graph: serializeWorkflowGraph({
          nodes: version.graph.nodes as unknown as WorkflowDefinitionNode[],
          edges: version.graph.edges as unknown as WorkflowDefinitionEdge[],
          viewport: version.graph.viewport ?? workflow.viewport,
          annotations: version.graph.annotations ?? [],
          globalVariables: version.graph.globalVariables ?? [],
          description: workflow.description,
        }),
        createdAt: BigInt(Date.parse(version.createdAt)),
        updatedAt: null,
      });
    });
    // Seed the newest published snapshot as the active run target (matches publish semantics).
    if (record.published.length > 0) {
      record.workflow.publishedSnapshotId = record.published[0].id;
    }
    return record;
  });
}

/** Titles of the toasts still on screen, ignoring the ones a test already dismissed. */
function activeToastTitles(): string[] {
  return toast
    .getToasts()
    .map((item) =>
      "title" in item && typeof item.title === "string" ? item.title : null,
    )
    .filter((title): title is string => title !== null);
}

/** Shell providers required by the workspace workflow editor (runtime + react-query). */
function renderEditor(
  ui?: ReactElement,
  state: FixtureState = createFixtureState(),
  configureHandlers?: (handlers: TestHandlers) => void,
  seedLibrary = true,
): RenderResult {
  if (seedLibrary) {
    seedDemoWorkflows(state);
  }
  // Model discovery resolves a Workspace, so the inspector needs one to ask against.
  state.projects = [{ id: "p1", name: "Demo" }];
  // Live Agent/Skill catalogs consumed by the workflow inspector's selectors.
  state.agents = [
    {
      id: "ag-architect",
      namespace: "local",
      name: "Architect",
      description: "role",
    },
    {
      id: "ag-planner",
      namespace: "local",
      name: "Planner",
      description: "role",
    },
    {
      id: "ag-researcher",
      namespace: "local",
      name: "Researcher",
      description: "role",
    },
    {
      id: "ag-implementer",
      namespace: "local",
      name: "Implementer",
      description: "role",
    },
    {
      id: "ag-reviewer",
      namespace: "local",
      name: "Reviewer",
      description: "role",
    },
    {
      id: "ag-tester",
      namespace: "local",
      name: "Tester",
      description: "role",
    },
    {
      id: "ag-debugger",
      namespace: "local",
      name: "Debugger",
      description: "role",
    },
    {
      id: "ag-documentation",
      namespace: "local",
      name: "Documentation Agent",
      description: "role",
    },
  ];
  state.skills = [
    {
      id: "openspec-verify-change",
      namespace: "local",
      name: "openspec-verify-change",
      description: "skill",
      source: { kind: "local" } as const,
      availability: "available",
    },
    {
      id: "openspec-archive-change",
      namespace: "local",
      name: "openspec-archive-change",
      description: "skill",
      source: { kind: "local" } as const,
      availability: "available",
    },
    {
      id: "openspec-explore",
      namespace: "local",
      name: "openspec-explore",
      description: "skill",
      source: { kind: "local" } as const,
      availability: "available",
    },
    {
      id: "cdase:sfmea_review",
      namespace: "local",
      name: "cdase:sfmea_review",
      description: "skill",
      source: { kind: "local" } as const,
      availability: "available",
    },
    {
      id: "missing-skill",
      namespace: "local",
      name: "missing-skill",
      description: "skill",
      source: { kind: "local" } as const,
      availability: "unavailable",
    },
  ];
  // Plugin-owned model catalog consumed by the workflow inspector's model selector.
  state.configOptions = [
    {
      id: "model",
      name: "Model",
      category: "model",
      type: "select",
      currentValue: "opencode/big-pickle",
      options: [
        { value: "opencode/big-pickle", name: "Big Pickle" },
        { value: "opencode/small-pickle", name: "Small Pickle" },
        { value: "deepseek/deepseek-v4-pro", name: "deepseek/deepseek-v4-pro" },
      ],
    },
  ];
  const clientHandlers: TestHandlers = createFixtureHandlers(state);
  const client = createTestClient(clientHandlers);
  configureHandlers?.(clientHandlers);
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  return render(
    <Wrapper>
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>
          <TooltipProvider>
            {ui ?? (
              <>
                <WorkflowEditorList />
                <WorkflowEditor />
              </>
            )}
          </TooltipProvider>
        </AppI18nProvider>
      </PlatformProvider>
    </Wrapper>,
  );
}

/** Reads graph-space coordinates exposed by the React Flow node card. */
function nodeGraphPosition(label: string): { x: string; y: string } {
  const node = screen.getByLabelText(label);
  return {
    x: `${node.dataset.x}px`,
    y: `${node.dataset.y}px`,
  };
}

/** jsdom never runs layout, and d3 drag needs a window-scoped event view. */
function windowedMouseEvent(
  type: "mousedown" | "mousemove" | "mouseup",
  init: { button?: number; clientX: number; clientY: number },
): MouseEvent {
  const event = new MouseEvent(type, {
    bubbles: true,
    cancelable: true,
    ...init,
  });
  Object.defineProperty(event, "view", { value: window });
  return event;
}

/**
 * Stubs React Flow node-wrapper offset sizes so `measured` reflects each card's
 * authored geometry: the iteration frame reports its rendered style size and
 * members report the standard card box. Other elements keep jsdom's zero layout.
 */
function stubNodeWrapperOffsetSize(): () => void {
  const sizeOf = (element: HTMLElement): { width: number; height: number } => {
    if (!element.hasAttribute("data-id")) {
      return { width: 0, height: 0 };
    }
    const frame = element.querySelector<HTMLElement>(
      "[data-workflow-iteration-frame]",
    );
    if (frame !== null) {
      return {
        width:
          Number.parseFloat(frame.style.width) || WORKFLOW_ITERATION_NODE_WIDTH,
        height:
          Number.parseFloat(frame.style.height) ||
          WORKFLOW_ITERATION_NODE_HEIGHT,
      };
    }
    return { width: WORKFLOW_NODE_WIDTH, height: WORKFLOW_NODE_INITIAL_HEIGHT };
  };
  // Capture jsdom's native descriptors up front: deleting the stub afterwards
  // would drop them entirely and leak `undefined` offset reads into later
  // tests sharing this worker (e.g. cmdk's deferred list measurement).
  const originalWidth = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    "offsetWidth",
  );
  const originalHeight = Object.getOwnPropertyDescriptor(
    HTMLElement.prototype,
    "offsetHeight",
  );
  Object.defineProperty(HTMLElement.prototype, "offsetWidth", {
    configurable: true,
    get() {
      return sizeOf(this as HTMLElement).width;
    },
  });
  Object.defineProperty(HTMLElement.prototype, "offsetHeight", {
    configurable: true,
    get() {
      return sizeOf(this as HTMLElement).height;
    },
  });
  return () => {
    if (originalWidth !== undefined) {
      Object.defineProperty(
        HTMLElement.prototype,
        "offsetWidth",
        originalWidth,
      );
    }
    if (originalHeight !== undefined) {
      Object.defineProperty(
        HTMLElement.prototype,
        "offsetHeight",
        originalHeight,
      );
    }
  };
}

/** Seeds a dedicated iteration showcase workflow the editor can open directly. */
function seedIterationShowcase(state: FixtureState): void {
  const showcase: DemoWorkflow = {
    id: "iteration-showcase",
    name: "迭代示例",
    description: "验证迭代区域的编辑交互。",
    updatedAt: "2026-09-16T10:00:00+08:00",
    viewport: { x: 32, y: 32, zoom: 1 },
    nodes: [
      {
        id: "start",
        type: "workflow",
        deletable: false,
        position: { x: 72, y: 286 },
        data: {
          kind: "start",
          title: "开始",
          description: "接收任务",
          input: "提取输入。",
          inputVariables: [{ name: "条目", valueType: "array" }],
        },
      },
      {
        id: "iter",
        type: "workflow",
        position: { x: 240, y: 120 },
        initialWidth: WORKFLOW_ITERATION_NODE_WIDTH,
        initialHeight: WORKFLOW_ITERATION_NODE_HEIGHT,
        data: {
          kind: "iteration",
          title: "评审迭代",
          description: "",
          iterationConfig: {
            iteratorSelector: ["start", "条目"],
            collectSelector: ["member", "输出"],
            errorStrategy: "fail",
            maxIterations: 10,
          },
        },
      },
      {
        id: "member",
        type: "workflow",
        parentId: "iter",
        position: { x: 120, y: 100 },
        data: {
          kind: "agent",
          title: "评审成员",
          description: "",
        },
      },
    ],
    edges: [
      { id: "e-start-iter", source: "start", target: "iter", type: "workflow" },
      {
        id: "e-iter-entry",
        source: "iter",
        sourceHandle: "iteration-entry",
        target: "member",
        type: "workflow",
      },
    ],
  };
  const now = BigInt(Date.parse(showcase.updatedAt));
  state.workflows.push({
    workflow: {
      id: showcase.id,
      namespace: "local",
      name: showcase.name,
      publishedSnapshotId: null,
      createdAt: now,
      updatedAt: now,
    },
    draft: {
      id: `snap-${showcase.id}`,
      workflowId: showcase.id,
      version: "draft",
      graph: serializeWorkflowGraph({
        nodes: showcase.nodes as unknown as WorkflowDefinitionNode[],
        edges: showcase.edges as unknown as WorkflowDefinitionEdge[],
        viewport: showcase.viewport,
        annotations: [],
        globalVariables: [],
        description: showcase.description,
      }),
      createdAt: now,
      updatedAt: now,
    },
    published: [],
  });
}

/** Seeds a small workflow whose condition branch edge carries a sourceHandle. */
function seedConditionDragWorkflow(state: FixtureState): void {
  const showcase: DemoWorkflow = {
    id: "condition-drag",
    name: "条件拖拽",
    description: "验证条件节点拖拽中的分支边锚点。",
    updatedAt: "2026-10-08T10:00:00+08:00",
    viewport: { x: 32, y: 32, zoom: 1 },
    nodes: [
      {
        id: "start",
        type: "workflow",
        deletable: false,
        position: { x: 72, y: 286 },
        data: {
          kind: "start",
          title: "开始",
          description: "",
          input: "",
          inputVariables: [],
        },
      },
      {
        id: "gate",
        type: "workflow",
        position: { x: 356, y: 188 },
        initialWidth: 320,
        initialHeight: WORKFLOW_NODE_INITIAL_HEIGHT,
        data: {
          kind: "condition",
          title: "门禁",
          description: "",
          conditionBranches: [
            {
              conditions: [
                { variable: "改动类型", operator: "contains", value: "源代码" },
              ],
            },
          ],
        },
      },
      {
        id: "worker",
        type: "workflow",
        position: { x: 760, y: 188 },
        initialWidth: WORKFLOW_NODE_WIDTH,
        initialHeight: WORKFLOW_NODE_INITIAL_HEIGHT,
        data: {
          kind: "agent",
          title: "执行",
          description: "",
        },
      },
    ],
    edges: [
      { id: "e-start-gate", source: "start", target: "gate", type: "workflow" },
      {
        id: "e-gate-worker",
        source: "gate",
        sourceHandle: "case-1",
        target: "worker",
        type: "workflow",
      },
    ],
  };
  const now = BigInt(Date.parse(showcase.updatedAt));
  state.workflows.push({
    workflow: {
      id: showcase.id,
      namespace: "local",
      name: showcase.name,
      publishedSnapshotId: null,
      createdAt: now,
      updatedAt: now,
    },
    draft: {
      id: `snap-${showcase.id}`,
      workflowId: showcase.id,
      version: "draft",
      graph: serializeWorkflowGraph({
        nodes: showcase.nodes as unknown as WorkflowDefinitionNode[],
        edges: showcase.edges as unknown as WorkflowDefinitionEdge[],
        viewport: showcase.viewport,
        annotations: [],
        globalVariables: [],
        description: showcase.description,
      }),
      createdAt: now,
      updatedAt: now,
    },
    published: [],
  });
}

/** Locates the React Flow viewport transform used for pan/zoom assertions. */
function flowViewport(): HTMLElement | null {
  return document.querySelector(".react-flow__viewport");
}

/** Opens one workflow action menu. */
function openWorkflowActions(name: string): void {
  fireEvent.click(
    screen.getByRole("button", { name: `打开${name}的操作菜单` }),
  );
}

describe("WorkflowEditor", () => {
  beforeEach(() => {
    useWorkflowEditorStore.setState({
      selectedWorkflowId: null,
      managerError: null,
      actions: null,
      importedWorkflowIds: [],
    });
    useUiStore.setState({
      sidebarCollapsed: false,
      workflowEditorOpen: false,
    });
    Object.defineProperty(HTMLElement.prototype, "clientWidth", {
      configurable: true,
      get() {
        return 800;
      },
    });
    Object.defineProperty(HTMLElement.prototype, "clientHeight", {
      configurable: true,
      get() {
        return 600;
      },
    });
  });

  afterEach(async () => {
    Reflect.deleteProperty(document, "elementFromPoint");
    // The workflow tree is still mounted until Testing Library's cleanup hook runs,
    // so resetting the shared i18n instance must flush its subscriber updates first.
    await act(() => appI18n.changeLanguage("zh-CN"));
  });

  it("loads the mock graph without workflow execution controls in the editor", async () => {
    renderEditor();

    expect(screen.queryByText("还没有工作流")).not.toBeInTheDocument();
    expect(await screen.findByText("代码审查工作流")).toBeInTheDocument();
    expect(await screen.findByLabelText("工作流画布")).toBeInTheDocument();
    expect(screen.queryByText("还没有工作流")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("separator", {
        name: "调整工作流列表宽度；双击恢复默认宽度",
      }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("separator", {
        name: "调整节点配置宽度；双击恢复默认宽度",
      }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "部署到项目" }),
    ).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "导出" })).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "打开代码审查工作流的操作菜单" }),
    ).toHaveClass("opacity-100");
    expect(
      screen.queryByRole("button", { name: "测试运行" }),
    ).not.toBeInTheDocument();
  });

  it("opens and edits an imported Agent with no executor and malformed bindings", async () => {
    const state = createFixtureState();
    const graph = JSON.stringify({
      nodes: [
        {
          id: "start",
          data: { kind: "start", title: "Start", description: "" },
        },
        {
          id: "agent",
          data: {
            kind: "agent",
            title: "Imported Agent",
            description: "",
            containerId: "absent",
            agentConfig: {
              schemaVersion: 3,
              prompt: "Imported prompt",
              skills: [null],
              mcps: {},
              roleId: null,
            },
          },
        },
      ],
      edges: [null, { source: "start", target: "agent" }],
    });
    state.workflows = [
      {
        workflow: {
          id: "imported",
          namespace: "local",
          name: "Imported workflow",
          publishedSnapshotId: null,
          createdAt: 1n,
          updatedAt: 1n,
        },
        draft: {
          id: "draft-imported",
          workflowId: "imported",
          version: "draft",
          graph,
          createdAt: 1n,
          updatedAt: 1n,
        },
        published: [],
      },
    ];
    const user = userEvent.setup();
    renderEditor(<WorkflowEditor />, state, undefined, false);
    const agent = await screen.findByLabelText("Agent节点: Imported Agent");
    await user.click(agent.closest(".react-flow__node") ?? agent);
    const prompt = await screen.findByLabelText("自定义 Prompt");
    expect(prompt).toHaveTextContent("Imported prompt");
    expect(screen.getByRole("button", { name: "角色" })).toHaveTextContent(
      "无角色",
    );
    await user.click(screen.getByRole("button", { name: "Agent" }));
    expect(
      await screen.findByLabelText("Agent节点: Agent 1"),
    ).toBeInTheDocument();
  });

  it("loads a draft with an unrenderable node kind by skipping that node", async () => {
    const state = createFixtureState();
    seedDemoWorkflows(state);
    // A workflow package ships its graph JSON verbatim, so a stored draft can carry a node
    // kind this Ora version has no renderer for. Loading it must keep the editor usable.
    const record = state.workflows.find(
      (candidate) => candidate.workflow.id === "code-review",
    );
    if (record === undefined) {
      throw new Error("the code-review fixture workflow is missing");
    }
    const graph = JSON.parse(record.draft.graph) as {
      nodes: Record<string, unknown>[];
      edges: unknown[];
    };
    const startNodeId = graph.nodes[0]?.id;
    if (typeof startNodeId !== "string") {
      throw new Error("the code-review fixture draft has no nodes");
    }
    graph.nodes.push({
      id: "route",
      type: "workflow",
      position: { x: 0, y: 0 },
      data: { kind: "router", title: "路由节点", description: "" },
    });
    graph.edges.push({
      id: "e-route",
      source: startNodeId,
      target: "route",
      type: "workflow",
    });
    record.draft.graph = JSON.stringify(graph);

    renderEditor(undefined, state, undefined, false);

    expect(await screen.findByLabelText("工作流画布")).toBeInTheDocument();
    expect(screen.queryByText("路由节点")).not.toBeInTheDocument();
    await waitFor(() => {
      expect(activeToastTitles()).toContain(
        "已跳过 1 个本版本无法渲染的节点（router）",
      );
    });
  });

  it("presents backend-derived unused nodes while preserving them in saved documents", async () => {
    const user = userEvent.setup();
    const saved: string[] = [];
    let spareId = "";
    renderEditor(undefined, createFixtureState(), (handlers) => {
      handlers.analyzeWorkflow = ({ graph }) => {
        const document = JSON.parse(graph) as {
          nodes: Array<{ id: string; data: { kind: string } }>;
        };
        spareId =
          document.nodes.find((node) => node.data.kind === "agent")?.id ?? "";
        return {
          unusedNodeIds: spareId === "" ? [] : [spareId],
          unrecognizedNodes:
            spareId === "" ? [] : [{ nodeId: "weird", kind: "hologram" }],
        };
      };
      const update = handlers.updateDraft!;
      handlers.updateDraft = (request, options) => {
        saved.push(request.graph);
        return update(request, options);
      };
    });
    await screen.findByText("有 1 个节点未参与运行");
    expect(
      screen.getByText(/有 1 个节点的类型无法识别（hologram）/),
    ).toBeInTheDocument();
    expect(screen.getByText("未参与运行")).toHaveAttribute(
      "title",
      "此节点或所属容器未从作用域入口连通，不会参与运行。",
    );
    const name = screen.getByLabelText("工作流名称");
    await user.clear(name);
    await user.type(name, "保留备用节点");
    await waitFor(() => expect(saved.length).toBeGreaterThan(0));
    const document = JSON.parse(saved.at(-1)!) as {
      nodes: Array<{ id: string; data: Record<string, unknown> }>;
    };
    const spare = document.nodes.find((node) => node.id === spareId);
    expect(spare).toBeDefined();
    expect(spare!.data).not.toHaveProperty("unused");
    expect(spare!.data).not.toHaveProperty("unusedNodeIds");
  });

  it("previews and activates a mock published workflow version", async () => {
    const user = userEvent.setup();
    renderEditor();

    await screen.findByLabelText("工作流画布");
    expect(screen.getByText(/生效中 · /)).toBeInTheDocument();
    await user.click(screen.getByLabelText("版本历史"));
    expect(screen.getByText("当前草稿")).toBeInTheDocument();

    const versionButtons = screen.getAllByRole("button", {
      name: /已发布版本|生效中/,
    });
    // Seed marks the first published snapshot as active; pick a non-active one.
    const inactiveButton = versionButtons.find(
      (button) => !/生效中/.test(button.getAttribute("aria-label") ?? ""),
    );
    expect(inactiveButton).toBeDefined();
    await user.click(inactiveButton!);
    expect(
      await screen.findByRole("button", { name: "设为生效版本" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("预览 2026-08-01T09:30:00.000 · 只读"),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("工作流名称")).toBeDisabled();
    expect(
      screen.queryByLabelText("输出节点: 输出报告"),
    ).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "设为生效版本" }));
    await waitFor(() => {
      expect(
        screen.queryByLabelText("输出节点: 输出报告"),
      ).not.toBeInTheDocument();
      expect(screen.getByLabelText("工作流名称")).toBeEnabled();
    });
    expect(screen.queryByText(/预览 .* · 只读/)).not.toBeInTheDocument();
    expect(
      screen.getByText("生效中 · 2026-08-01T09:30:00.000"),
    ).toBeInTheDocument();

    await user.click(screen.getByLabelText("版本历史"));
    await waitFor(() => {
      expect(screen.getByText("生效中")).toBeInTheDocument();
    });
    // Active version preview offers a status hint, not a redundant activate action.
    const activePreview = screen.getByRole("button", {
      name: /2026-08-01T09:30:00\.000.*生效中/,
    });
    await user.click(activePreview);
    expect(
      screen.queryByRole("button", { name: "设为生效版本" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByText("这是当前生效的版本，运行工作流时会使用它。"),
    ).toBeInTheDocument();
    expect(
      screen.getByText("预览 2026-08-01T09:30:00.000 · 只读"),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "返回草稿" }));
    expect(screen.getByLabelText("工作流名称")).toBeEnabled();
    expect(screen.queryByText(/预览 .* · 只读/)).not.toBeInTheDocument();
  }, 15_000);

  it("opens the publish dialog from the draft row in version history", async () => {
    const user = userEvent.setup();
    renderEditor();

    await screen.findByLabelText("工作流画布");
    await user.click(screen.getByLabelText("版本历史"));
    await user.click(screen.getByRole("button", { name: "发布当前草稿" }));

    expect(
      await screen.findByRole("alertdialog", { name: "发布工作流" }),
    ).toBeInTheDocument();
  });

  it("zooms around the pointer with the mouse wheel", async () => {
    renderEditor();
    await screen.findByLabelText("工作流画布");
    const pane = document.querySelector(".react-flow__pane");
    expect(pane).not.toBeNull();

    expect(screen.getByText("100%")).toBeInTheDocument();
    fireEvent.wheel(pane!, { deltaY: -200, clientX: 240, clientY: 180 });

    await waitFor(() => {
      expect(screen.queryByText("100%")).not.toBeInTheDocument();
    });
  });

  it("exposes canvas zoom controls and resets the React Flow viewport", async () => {
    const user = userEvent.setup();
    renderEditor();
    await screen.findByLabelText("工作流画布");
    // React Flow applies the initial viewport transform asynchronously after mount.
    await waitFor(() => {
      expect(flowViewport()?.style.transform).toContain("translate(32px,32px)");
    });

    expect(screen.getByText("100%")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "放大画布" }));
    await waitFor(() => {
      expect(screen.queryByText("100%")).not.toBeInTheDocument();
    });

    expect(
      screen.getByRole("button", { name: "显示完整工作流" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("工作流小地图")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "重置画布视图" }));
    await waitFor(() => {
      expect(screen.getByText("100%")).toBeInTheDocument();
      expect(flowViewport()?.style.transform).toContain("translate(32px,32px)");
    });
  });

  it("adds and edits annotations and switches between pointer and hand modes", async () => {
    const user = userEvent.setup();
    renderEditor();
    const canvas = await screen.findByLabelText("工作流画布");
    vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
      ...canvas.getBoundingClientRect(),
      left: 0,
      top: 0,
      width: 800,
      height: 600,
      right: 800,
      bottom: 600,
    });

    const pointer = screen.getByRole("button", { name: "指针模式" });
    const hand = screen.getByRole("button", { name: "手型模式" });
    expect(pointer).toHaveAttribute("aria-pressed", "true");
    await user.click(hand);
    expect(hand).toHaveAttribute("aria-pressed", "true");
    expect(canvas.querySelector(".workflow-flow")).toHaveAttribute(
      "data-interaction-mode",
      "hand",
    );
    expect(
      screen.getByLabelText("Agent节点: 理解改动").closest(".react-flow__node"),
    ).toHaveClass("draggable");
    await user.click(pointer);

    const addAnnotation = screen.getByRole("button", { name: "添加注释" });
    const annotationIcon = addAnnotation.querySelector("svg");
    expect(annotationIcon).toHaveClass("size-5");
    await user.click(addAnnotation);
    const annotation = await screen.findByLabelText("注释内容");
    expect(annotation.closest(".nodrag")).toBeNull();
    expect(annotation.closest("[data-workflow-annotation-id]")).toHaveClass(
      "cursor-grab",
      "active:cursor-grabbing",
    );
    expect(annotation.closest(".react-flow__node")).toHaveStyle({ zIndex: 0 });
    expect(
      screen.getByLabelText("Agent节点: 理解改动").closest(".react-flow__node"),
    ).toHaveStyle({ zIndex: 1 });
    await user.click(annotation);
    const note = await screen.findByRole("textbox", { name: "注释内容" });
    fireEvent.change(note, { target: { value: "检查并行分支" } });
    await waitFor(() => {
      expect(screen.getByRole("textbox", { name: "注释内容" })).toHaveValue(
        "检查并行分支",
      );
    });
    await user.click(screen.getByRole("button", { name: "删除注释" }));
    await waitFor(() => {
      expect(
        screen.queryByRole("textbox", { name: "注释内容" }),
      ).not.toBeInTheDocument();
    });
  });

  it("does not pan from the panel-resize guard zones at canvas edges", async () => {
    renderEditor();
    const canvas = await screen.findByLabelText("工作流画布");
    vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
      ...canvas.getBoundingClientRect(),
      left: 0,
      top: 0,
      width: 800,
      height: 600,
      right: 800,
      bottom: 600,
    });
    const viewport = flowViewport();
    const before = viewport?.style.transform;

    fireEvent.pointerDown(canvas, {
      button: 0,
      clientX: 6,
      clientY: 200,
      pointerId: 1,
      bubbles: true,
    });

    expect(viewport?.style.transform).toBe(before);
  });

  it("keeps workflow node positions under parent graph state", async () => {
    renderEditor();
    await screen.findByLabelText("开始节点: 开始");

    expect(nodeGraphPosition("开始节点: 开始")).toEqual({
      x: "72px",
      y: "286px",
    });
    expect(nodeGraphPosition("Agent节点: 理解改动")).toEqual({
      x: "356px",
      y: "188px",
    });
  });

  it("tracks the pointer live while dragging a node and commits once on drop", async () => {
    const user = userEvent.setup();
    renderEditor();
    const card = await screen.findByLabelText("Agent节点: 理解改动");
    const flowNode = card.closest<HTMLElement>(".react-flow__node");
    expect(flowNode).not.toBeNull();
    expect(nodeGraphPosition("Agent节点: 理解改动")).toEqual({
      x: "356px",
      y: "188px",
    });

    await act(async () => {
      flowNode!.dispatchEvent(
        windowedMouseEvent("mousedown", {
          button: 0,
          clientX: 400,
          clientY: 250,
        }),
      );
    });
    // The first move starts the gesture and re-snaps the card to the grid.
    // Mid-gesture the card must already track the pointer, before any drop
    // commit reaches editor state.
    await act(async () => {
      window.dispatchEvent(
        windowedMouseEvent("mousemove", {
          button: 0,
          clientX: 410,
          clientY: 255,
        }),
      );
    });
    expect(nodeGraphPosition("Agent节点: 理解改动")).toEqual({
      x: "360px",
      y: "180px",
    });
    // Zoom 1 keeps screen deltas 1:1 in graph space; positions snap to 20px.
    await act(async () => {
      window.dispatchEvent(
        windowedMouseEvent("mousemove", {
          button: 0,
          clientX: 510,
          clientY: 315,
        }),
      );
    });
    expect(nodeGraphPosition("Agent节点: 理解改动")).toEqual({
      x: "460px",
      y: "240px",
    });

    await act(async () => {
      window.dispatchEvent(
        windowedMouseEvent("mouseup", {
          button: 0,
          clientX: 510,
          clientY: 315,
        }),
      );
    });
    // The drop commits the same geometry the pointer previewed.
    expect(nodeGraphPosition("Agent节点: 理解改动")).toEqual({
      x: "460px",
      y: "240px",
    });

    // The whole gesture is exactly one undoable history step.
    await user.click(screen.getByRole("button", { name: "撤销" }));
    await waitFor(() => {
      expect(nodeGraphPosition("Agent节点: 理解改动")).toEqual({
        x: "356px",
        y: "188px",
      });
    });
  });

  it("keeps condition branch edges attached while dragging the condition node", async () => {
    const state = createFixtureState();
    seedDemoWorkflows(state);
    seedConditionDragWorkflow(state);
    useWorkflowEditorStore.setState({ selectedWorkflowId: "condition-drag" });
    const restoreOffsetSize = stubNodeWrapperOffsetSize();
    renderEditor(undefined, state, undefined, false);

    try {
      const card = await screen.findByLabelText("条件分支节点: 门禁");
      const flowNode = card.closest<HTMLElement>(".react-flow__node");
      expect(flowNode).not.toBeNull();
      // The branch edge anchors on a measured handle id (case-1) that the
      // initial static handle declaration does not contain.
      expect(
        await screen.findByTestId("rf__edge-e-gate-worker"),
      ).toBeInTheDocument();

      await act(async () => {
        flowNode!.dispatchEvent(
          windowedMouseEvent("mousedown", {
            button: 0,
            clientX: 400,
            clientY: 250,
          }),
        );
      });
      await act(async () => {
        window.dispatchEvent(
          windowedMouseEvent("mousemove", {
            button: 0,
            clientX: 410,
            clientY: 255,
          }),
        );
      });
      await act(async () => {
        window.dispatchEvent(
          windowedMouseEvent("mousemove", {
            button: 0,
            clientX: 520,
            clientY: 330,
          }),
        );
      });

      // Mid-drag the branch edge must stay attached to its measured handle.
      expect(screen.getByTestId("rf__edge-e-gate-worker")).toBeInTheDocument();
      expect(nodeGraphPosition("条件分支节点: 门禁")).not.toEqual({
        x: "356px",
        y: "188px",
      });

      await act(async () => {
        window.dispatchEvent(
          windowedMouseEvent("mouseup", {
            button: 0,
            clientX: 520,
            clientY: 330,
          }),
        );
      });
      expect(screen.getByTestId("rf__edge-e-gate-worker")).toBeInTheDocument();
    } finally {
      restoreOffsetSize();
    }
  });

  it("keeps each workflow port independently visible without node-wide hover styles", async () => {
    renderEditor();
    const input = await screen.findByLabelText("连接到理解改动");
    const output = screen.getByLabelText("从理解改动开始连接");

    expect(input).toHaveClass("workflow-port", "workflow-port-input");
    expect(output).toHaveClass("workflow-port", "workflow-port-output");
    expect(input).not.toHaveClass("opacity-0");
    expect(output).not.toHaveClass("opacity-0");
    expect(input.className).not.toContain("group-hover");
    expect(output.className).not.toContain("group-hover");
  });

  it("collapses node configuration after a stationary blank-canvas click", async () => {
    const user = userEvent.setup();
    renderEditor();
    const startNode = await screen.findByLabelText("开始节点: 开始");
    const flowNode = startNode.closest(".react-flow__node") ?? startNode;

    await user.click(flowNode);
    expect(
      screen.getByRole("button", { name: "收起节点配置" }),
    ).toBeInTheDocument();

    const pane = document.querySelector(".react-flow__pane");
    expect(pane).not.toBeNull();
    await user.click(pane!);

    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: "收起节点配置" }),
      ).not.toBeInTheDocument();
    });
  });

  it("reserves a visible trailing action column while the workflow library narrows", async () => {
    renderEditor();
    const actionButton = await screen.findByRole("button", {
      name: "打开代码审查工作流的操作菜单",
    });
    const workflowItem = actionButton.parentElement;
    const workflowManager = actionButton.closest("div.flex-col");
    const selectionButton = screen
      .getByText("代码审查工作流")
      .closest("button");

    expect(workflowManager).toHaveClass("min-w-0", "overflow-hidden");
    expect(workflowItem).toHaveClass("flex");
    expect(selectionButton).toHaveClass("min-w-0", "flex-1");
    expect(actionButton).toHaveClass("shrink-0");
    expect(actionButton).not.toHaveClass("absolute");
  });

  it("closes an open workflow action menu when its trigger is clicked again", async () => {
    renderEditor();
    const actionButton = await screen.findByRole("button", {
      name: "打开代码审查工作流的操作菜单",
    });

    fireEvent.click(actionButton);
    expect(screen.getByRole("menuitem", { name: "复制" })).toBeInTheDocument();

    fireEvent.click(actionButton);
    await waitFor(() => {
      expect(
        screen.queryByRole("menuitem", { name: "复制" }),
      ).not.toBeInTheDocument();
    });
  });

  it("closes node configuration with its button or Escape", async () => {
    const user = userEvent.setup();
    renderEditor();
    const startNode = await screen.findByLabelText("开始节点: 开始");
    const flowNode = startNode.closest(".react-flow__node") ?? startNode;

    await user.click(flowNode);
    await user.click(screen.getByRole("button", { name: "收起节点配置" }));
    expect(
      screen.queryByRole("button", { name: "收起节点配置" }),
    ).not.toBeInTheDocument();

    await user.click(flowNode);
    fireEvent.keyDown(startNode, { key: "Escape" });

    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: "收起节点配置" }),
      ).not.toBeInTheDocument();
    });
  });

  it("switches workflows from the manager and adds nodes from the bottom dock", async () => {
    const user = userEvent.setup();
    renderEditor();

    const releaseWorkflow = await screen.findByText("发布准备检查");
    await user.click(releaseWorkflow.closest("button")!);

    expect(screen.getByDisplayValue("发布准备检查")).toBeInTheDocument();
    expect(screen.getByLabelText("添加工作流节点")).toBeInTheDocument();
    // The start entry stays visible in the dock but is disabled while the
    // required start node already exists on the canvas.
    expect(screen.getByRole("button", { name: "开始" })).toBeDisabled();
    const canvas = screen.getByLabelText("工作流画布");
    vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
      ...canvas.getBoundingClientRect(),
      left: 0,
      top: 0,
      width: 800,
      height: 600,
      right: 800,
      bottom: 600,
    });

    await user.click(screen.getByRole("button", { name: "Agent" }));

    // The card and inspector header share the generated title; editing stays
    // hidden until the user double-clicks that title.
    expect(screen.getByLabelText("Agent节点: Agent 1")).toHaveTextContent(
      "Agent 1",
    );
    expect(
      screen.getByRole("heading", { name: "Agent 1" }),
    ).toBeInTheDocument();
    expect(screen.queryByLabelText("名称")).not.toBeInTheDocument();
    expect(nodeGraphPosition("Agent节点: Agent 1")).toEqual({
      x: "260px",
      y: "200px",
    });
  });

  it("adds, configures, and deletes an executable Loop container as one group", async () => {
    const user = userEvent.setup();
    renderEditor();

    await user.click(await screen.findByRole("button", { name: "循环" }));

    const loop = screen.getByLabelText("循环节点: 循环 1");
    expect(loop).toBeInTheDocument();
    expect(screen.getByLabelText("开始节点: 轮次开始")).toBeInTheDocument();
    expect(screen.getByLabelText("Agent节点: 循环 Agent")).toBeInTheDocument();
    expect(screen.getByLabelText("工作流画布")).toHaveAttribute(
      "data-workflow-edge-count",
      "7",
    );

    const maximumRounds = screen.getByLabelText("最大轮次");
    expect(maximumRounds).toHaveValue(3);
    fireEvent.change(maximumRounds, { target: { value: "5" } });
    expect(screen.getByLabelText("最大轮次")).toHaveValue(5);
    fireEvent.change(screen.getByLabelText("初始值"), {
      target: { value: "draft" },
    });
    expect(screen.getByLabelText("初始值")).toHaveValue("draft");

    await user.click(loop.closest(".react-flow__node") ?? loop);
    await user.click(screen.getByRole("button", { name: "删除循环 1" }));

    await waitFor(() => {
      expect(
        screen.queryByLabelText("循环节点: 循环 1"),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByLabelText("开始节点: 轮次开始"),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByLabelText("Agent节点: 循环 Agent"),
      ).not.toBeInTheDocument();
    });
  });

  it("drags a node type from the dock to the chosen canvas position", async () => {
    renderEditor();
    const canvas = await screen.findByLabelText("工作流画布");
    vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
      ...canvas.getBoundingClientRect(),
      left: 0,
      top: 0,
      width: 800,
      height: 600,
      right: 800,
      bottom: 600,
    });
    const agentButton = screen.getByRole("button", { name: "Agent" });
    agentButton.setPointerCapture = () => {};

    expect(canvas).not.toContainElement(agentButton);
    fireEvent.pointerDown(agentButton, {
      button: 0,
      isPrimary: true,
      pointerId: 1,
      clientX: 400,
      clientY: 650,
    });
    fireEvent.pointerMove(agentButton, {
      isPrimary: true,
      pointerId: 1,
      clientX: 500,
      clientY: 350,
    });

    expect(document.querySelector("[data-workflow-node-preview]")).toHaveStyle({
      left: "500px",
      top: "350px",
      transform: "translate(-50%, -50%)",
    });

    fireEvent.pointerUp(agentButton, {
      isPrimary: true,
      pointerId: 1,
      clientX: 500,
      clientY: 350,
    });
    fireEvent.click(agentButton);

    expect(
      document.querySelector("[data-workflow-node-preview]"),
    ).not.toBeInTheDocument();
    expect(nodeGraphPosition("Agent节点: Agent 1")).toEqual({
      x: "360px",
      y: "260px",
    });
    expect(screen.queryByText("释放以添加节点")).not.toBeInTheDocument();
  });

  it("deletes workflow connections by double-click or keyboard", async () => {
    const user = userEvent.setup();
    renderEditor();

    const connection = await screen.findByRole("button", {
      name: "Edge from start to understand",
    });
    await user.dblClick(connection);

    await waitFor(() => {
      expect(
        screen.queryByRole("button", {
          name: "Edge from start to understand",
        }),
      ).not.toBeInTheDocument();
    });

    const keyboardConnection = screen.getByRole("button", {
      name: "Edge from understand to quality",
    });
    await user.click(keyboardConnection);
    await user.keyboard("{Delete}");

    await waitFor(() => {
      expect(
        screen.queryByRole("button", {
          name: "Edge from understand to quality",
        }),
      ).not.toBeInTheDocument();
    });
  });

  it("restores each workflow from its React Flow viewport snapshot", async () => {
    const user = userEvent.setup();
    renderEditor();
    await screen.findByLabelText("工作流画布");

    await user.click(screen.getByRole("button", { name: "放大画布" }));
    await waitFor(() => {
      expect(screen.queryByText("100%")).not.toBeInTheDocument();
    });
    const editedViewport = flowViewport()?.style.transform;

    // Switching workflows force-flushes the draft, including the live viewport.
    await user.click(screen.getByText("发布准备检查").closest("button")!);
    await waitFor(() => {
      expect(flowViewport()?.style.transform).toContain("translate(32px,32px)");
    });

    await user.click(screen.getByText("代码审查工作流").closest("button")!);
    await waitFor(() => {
      expect(flowViewport()?.style.transform).toBe(editedViewport);
    });
  });

  it("uses React Flow deletion to remove a node and its incident edges", async () => {
    const user = userEvent.setup();
    renderEditor();

    const node = await screen.findByLabelText("Agent节点: 理解改动");
    expect(
      screen.getByRole("button", {
        name: "Edge from start to understand",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", {
        name: "Edge from understand to quality",
      }),
    ).toBeInTheDocument();

    await user.click(node.closest(".react-flow__node") ?? node);
    await user.click(screen.getByRole("button", { name: "删除理解改动" }));

    await waitFor(() => {
      expect(
        screen.queryByLabelText("Agent节点: 理解改动"),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", {
          name: "Edge from start to understand",
        }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", {
          name: "Edge from understand to quality",
        }),
      ).not.toBeInTheDocument();
    });
  });

  it("box-selects multiple nodes with a left drag and deletes them together", async () => {
    const user = userEvent.setup();
    renderEditor();
    const canvas = await screen.findByLabelText("工作流画布");
    vi.spyOn(canvas, "getBoundingClientRect").mockReturnValue({
      ...canvas.getBoundingClientRect(),
      left: 0,
      top: 0,
      width: 800,
      height: 600,
      right: 800,
      bottom: 600,
    });
    const pane = canvas.querySelector<HTMLElement>(".react-flow__pane");
    expect(pane).not.toBeNull();
    pane!.setPointerCapture = () => {};

    fireEvent.pointerDown(pane!, {
      button: 0,
      isPrimary: true,
      pointerId: 1,
      clientX: 50,
      clientY: 50,
      bubbles: true,
    });
    fireEvent.pointerMove(pane!, {
      button: 0,
      isPrimary: true,
      pointerId: 1,
      clientX: 950,
      clientY: 550,
      bubbles: true,
    });
    fireEvent.pointerUp(pane!, {
      button: 0,
      isPrimary: true,
      pointerId: 1,
      clientX: 950,
      clientY: 550,
      bubbles: true,
    });

    expect(
      canvas.querySelectorAll(".react-flow__node.selected").length,
    ).toBeGreaterThan(1);
    await user.keyboard("{Delete}");
    await waitFor(() => {
      expect(
        screen.queryByLabelText("Agent节点: 理解改动"),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByLabelText("条件分支节点: 质量门禁"),
      ).not.toBeInTheDocument();
      // The required start node is not deletable and survives the batch delete.
      expect(screen.getByLabelText("开始节点: 开始")).toBeInTheDocument();
    });
  });

  it("uses React Flow deletable state to protect the required start node", async () => {
    const user = userEvent.setup();
    renderEditor();

    const startNode = await screen.findByLabelText("开始节点: 开始");
    await user.click(startNode.closest(".react-flow__node") ?? startNode);

    expect(
      screen.queryByRole("button", { name: "删除开始" }),
    ).not.toBeInTheDocument();
    await user.keyboard("{Delete}");
    expect(screen.getByLabelText("开始节点: 开始")).toBeInTheDocument();
  });

  it("edits the existing Agent node through its structured execution contract", async () => {
    const user = userEvent.setup();
    renderEditor();

    const reviewNode = await screen.findByLabelText("Agent节点: 审查 Agent");
    expect(within(reviewNode).getByText("review")).toBeInTheDocument();
    await user.click(reviewNode.closest(".react-flow__node") ?? reviewNode);

    expect(screen.getByLabelText("Agent 模型")).toBeInTheDocument();
    expect(screen.getByLabelText("角色")).toHaveTextContent("Reviewer");
    expect(screen.getAllByText("必需 Skill")).toHaveLength(2);
    expect(screen.getByLabelText("自定义 Prompt")).toHaveTextContent(
      "按严重程度整理问题，并给出定位与修复建议。",
    );
    expect(screen.queryByText("输入上下文")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("项目权限")).not.toBeInTheDocument();
    expect(
      screen.getByRole("switch", { name: "结构化输出" }),
    ).not.toBeChecked();
    expect(screen.queryByText("使用输出策略")).not.toBeInTheDocument();
    await waitFor(() => {
      expect(screen.getByLabelText("Agent 模型")).toHaveTextContent(
        "Big Pickle",
      );
    });
    const configuredParameters = within(reviewNode).getByLabelText("配置参数");
    expect(configuredParameters).toHaveTextContent("角色Reviewer");
    expect(configuredParameters).toHaveTextContent(
      `${AGENT_REF.codeagentcli} · opencode/big-pickle`,
    );
    expect(configuredParameters).toHaveTextContent(
      "必需 Skillcode-defect-scan",
    );
    expect(configuredParameters).not.toHaveTextContent(
      "按严重程度整理问题，并给出定位与修复建议。",
    );
  });

  it("opens system variables from the left toolbar as read-only cards", async () => {
    const user = userEvent.setup();
    renderEditor();

    const toolbar = await screen.findByRole("toolbar", {
      name: "画布工具",
    });
    const variablesButton = within(toolbar).getByRole("button", {
      name: "全局变量",
    });
    const annotationButton = within(toolbar).getByRole("button", {
      name: "添加注释",
    });
    expect(
      variablesButton.compareDocumentPosition(annotationButton) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).not.toBe(0);

    await user.click(variablesButton);
    const dialog = await screen.findByRole("dialog");
    expect(within(dialog).getByText("系统变量")).toBeInTheDocument();
    expect(within(dialog).getByText("sys.workflow_id")).toBeInTheDocument();
    expect(within(dialog).getByText("当前工作流 ID")).toBeInTheDocument();
    expect(within(dialog).getByText("sys.timestamp")).toBeInTheDocument();
    expect(
      within(dialog).getByText("应用开始运行时的时间戳"),
    ).toBeInTheDocument();
    expect(within(dialog).queryByText("sys.user_id")).not.toBeInTheDocument();

    await user.click(
      within(dialog).getByRole("button", { name: "添加全局变量" }),
    );
    const customName = within(dialog).getByPlaceholderText(
      "global.variable_name",
    );
    const customValue = within(dialog).getByPlaceholderText("例如：text");
    expect(customName).toHaveValue("");
    expect(customValue).toHaveValue("");
    expect(within(dialog).getByRole("button", { name: "保存" })).toBeDisabled();

    await user.type(customName, "global.region");
    await user.type(customValue, "上海");
    expect(within(dialog).getByRole("button", { name: "保存" })).toBeEnabled();

    await user.click(within(dialog).getByLabelText("全局变量 3 类型"));
    await user.click(await screen.findByRole("option", { name: "integer" }));
    expect(customValue).toHaveValue("");
    expect(customValue).toHaveAttribute("placeholder", "例如：1");
    await user.type(customValue, "1.5");
    expect(customValue).toHaveAttribute("aria-invalid", "true");
    expect(
      within(dialog).getByText("值与 integer 类型不匹配。"),
    ).toBeInTheDocument();
    expect(within(dialog).getByRole("button", { name: "保存" })).toBeDisabled();
    await user.clear(customValue);
    await user.type(customValue, "2");
    expect(within(dialog).getByRole("button", { name: "保存" })).toBeEnabled();
  });

  it("limits node descriptions to 30 characters and shows their count", async () => {
    const user = userEvent.setup();
    renderEditor();

    const reviewNode = await screen.findByLabelText("Agent节点: 审查 Agent");
    await user.click(reviewNode.closest(".react-flow__node") ?? reviewNode);
    const description = screen.getByLabelText("说明");

    expect(description).toHaveAttribute("maxlength", "30");
    expect(screen.getByText("9/30")).toBeInTheDocument();
    fireEvent.change(description, {
      target: { value: "1234567890123456789012345678901" },
    });

    expect(screen.getByLabelText("说明")).toHaveValue(
      "123456789012345678901234567890",
    );
    expect(screen.getByText("30/30")).toBeInTheDocument();
  });

  it("searches Agent models and roles before updating their selections", async () => {
    const user = userEvent.setup();
    renderEditor();

    const reviewNode = await screen.findByLabelText("Agent节点: 审查 Agent");
    await user.click(reviewNode.closest(".react-flow__node") ?? reviewNode);

    await user.click(screen.getByLabelText("Agent 模型"));
    const modelSearch = screen.getByLabelText("搜索可用 Agent 模型");
    await user.type(modelSearch, "pickle");
    await user.click(
      await screen.findByRole("option", {
        name: "Big Pickle",
      }),
    );
    expect(screen.getByLabelText("Agent 模型")).toHaveTextContent("Big Pickle");

    await user.click(screen.getByLabelText("角色"));
    const roleSearch = screen.getByLabelText("搜索可用角色");
    await user.type(roleSearch, "tester");
    await user.click(screen.getByRole("option", { name: "Tester" }));
    expect(screen.getByLabelText("角色")).toHaveTextContent("Tester");
  });

  it("keeps a manually switched Agent CLI when that CLI reports no models", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    state.agents = [
      {
        id: "Architect",
        namespace: "local",
        name: "架构师",
        description: "role",
      },
      {
        id: "Planner",
        namespace: "local",
        name: "规划师",
        description: "role",
      },
      {
        id: "Researcher",
        namespace: "local",
        name: "研究员",
        description: "role",
      },
      {
        id: "Implementer",
        namespace: "local",
        name: "实施者",
        description: "role",
      },
      {
        id: "Reviewer",
        namespace: "local",
        name: "审查员",
        description: "role",
      },
      { id: "Tester", namespace: "local", name: "测试员", description: "role" },
      {
        id: "Debugger",
        namespace: "local",
        name: "调试员",
        description: "role",
      },
      {
        id: "Documentation Agent",
        namespace: "local",
        name: "文档专员",
        description: "role",
      },
    ];
    state.skills = [
      {
        id: "openspec-verify-change",
        namespace: "local",
        name: "openspec-verify-change",
        description: "skill",
        source: { kind: "local" } as const,
        availability: "available",
      },
    ];
    // NGA exists as a CLI but reports no model catalog, so
    // picking it must keep the node on NGA instead of snapping back to the
    // first CLI with discovered models.
    state.agentModelsByCli = { [AGENT_REF.nga]: null };
    renderEditor(<WorkflowEditor />, state);

    const reviewNode = await screen.findByLabelText("Agent节点: 审查 Agent");
    await user.click(reviewNode.closest(".react-flow__node") ?? reviewNode);

    const modelSelect = screen.getByLabelText("Agent 模型");
    await waitFor(() => expect(modelSelect).toBeEnabled());
    await user.click(modelSelect);
    await user.click(screen.getByRole("option", { name: /NGA/ }));

    expect(screen.getByLabelText("Agent 模型")).toHaveTextContent(/NGA/);
    expect(screen.getByText("没有可用 Agent 模型")).toBeInTheDocument();

    await user.keyboard("{Escape}");
    await waitFor(() => {
      expect(screen.getByLabelText("Agent 模型")).toHaveTextContent(/NGA/);
    });
  });

  it("uses the backend catalog for a newly added Agent model", async () => {
    const user = userEvent.setup();
    renderEditor();

    const reviewNode = await screen.findByLabelText("Agent节点: 审查 Agent");
    await user.click(reviewNode.closest(".react-flow__node") ?? reviewNode);
    const modelSelect = screen.getByLabelText("Agent 模型");
    await waitFor(() => expect(modelSelect).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Agent" }));

    expect(
      await screen.findByLabelText("Agent节点: Agent 1"),
    ).toBeInTheDocument();
    expect(
      screen.getAllByText(`${AGENT_REF.opencode} · opencode/big-pickle`).length,
    ).toBeGreaterThan(0);
  });

  it("adds, disables, and removes configured Agent Skills", async () => {
    const user = userEvent.setup();
    renderEditor();

    const reviewNode = await screen.findByLabelText("Agent节点: 审查 Agent");
    await user.click(reviewNode.closest(".react-flow__node") ?? reviewNode);
    const existingSkillSwitch = screen.getByRole("switch", {
      name: "启用或禁用 code-defect-scan",
    });
    expect(existingSkillSwitch).toBeChecked();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "添加 Skill" }));
    expect(screen.queryByText("missing-skill")).not.toBeInTheDocument();
    const skillSearch = screen.getByLabelText("搜索可添加的 Skill");
    await user.type(skillSearch, "archive");
    await user.click(screen.getByText("openspec-archive-change"));

    const archiveSwitch = screen.getByRole("switch", {
      name: "启用或禁用 openspec-archive-change",
    });
    expect(archiveSwitch).toBeChecked();
    await user.click(archiveSwitch);
    expect(archiveSwitch).not.toBeChecked();

    await user.click(
      screen.getByRole("button", {
        name: "移除 openspec-archive-change",
      }),
    );
    expect(
      screen.queryByText("openspec-archive-change"),
    ).not.toBeInTheDocument();
  });

  it("routes inspector deletion through the shared React Flow store", async () => {
    const user = userEvent.setup();
    renderEditor();

    const reviewNode = await screen.findByLabelText("Agent节点: 审查 Agent");
    await user.click(reviewNode.closest(".react-flow__node") ?? reviewNode);
    await user.click(screen.getByRole("button", { name: "删除节点" }));

    await waitFor(() => {
      expect(
        screen.queryByLabelText("Agent节点: 审查 Agent"),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", {
          name: "Edge from quality to review",
        }),
      ).not.toBeInTheDocument();
      expect(
        screen.queryByRole("button", {
          name: "Edge from review to output",
        }),
      ).not.toBeInTheDocument();
    });
  });

  it("shows React Flow reconnect controls after selecting an edge", async () => {
    const user = userEvent.setup();
    renderEditor();

    const connection = await screen.findByRole("button", {
      name: "Edge from start to understand",
    });
    await user.click(connection);

    await waitFor(() => {
      expect(
        document.querySelector(".react-flow__edgeupdater-source"),
      ).not.toBeNull();
      expect(
        document.querySelector(".react-flow__edgeupdater-target"),
      ).not.toBeNull();
    });
  });

  it("creates a workflow from the left manager and allows renaming it", async () => {
    const user = userEvent.setup();
    renderEditor();

    await screen.findByText("代码审查工作流");
    await screen.findByLabelText("工作流画布");
    // Match the row-menu helper: open the Base UI menu with a single click event.
    fireEvent.click(screen.getByRole("button", { name: "新建或导入工作流" }));
    await user.click(
      await screen.findByRole("menuitem", { name: /新建工作流/ }),
    );
    const createDialog = await screen.findByRole("alertdialog", {
      name: "新建工作流",
    });
    const createNameInput = within(createDialog).getByLabelText("工作流名称");
    await user.type(createNameInput, "发布复盘");
    await user.click(
      within(createDialog).getByRole("button", { name: "新建工作流" }),
    );

    await waitFor(() => {
      expect(screen.getByDisplayValue("发布复盘")).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "发布复盘" }),
      ).toBeInTheDocument();
    });
    const created = screen.getByRole("button", { name: "发布复盘" });
    const previous = screen.getByRole("button", { name: "代码审查工作流" });
    expect(
      created.compareDocumentPosition(previous) &
        Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBe(Node.DOCUMENT_POSITION_FOLLOWING);
    expect(
      screen.queryByDisplayValue("代码审查工作流"),
    ).not.toBeInTheDocument();
    expect(screen.getByText("暂未发布")).toBeInTheDocument();

    openWorkflowActions("发布复盘");
    await user.click(screen.getByRole("menuitem", { name: "重命名" }));
    const renameDialog = await screen.findByRole("alertdialog", {
      name: "重命名“发布复盘”",
    });
    const renameNameInput = within(renameDialog).getByDisplayValue("发布复盘");
    await user.clear(renameNameInput);
    await user.type(renameNameInput, "发布复盘 v2");
    await user.click(
      within(renameDialog).getByRole("button", { name: "重命名" }),
    );

    await waitFor(() => {
      expect(screen.getByDisplayValue("发布复盘 v2")).toBeInTheDocument();
    });
  });

  it("clears the library search so a created workflow stays visible and selected", async () => {
    const user = userEvent.setup();
    renderEditor();

    await screen.findByText("代码审查工作流");
    fireEvent.change(screen.getByPlaceholderText("搜索工作流"), {
      target: { value: "代码审查工作流" },
    });
    expect(
      screen.queryByRole("button", { name: "错开并行演示" }),
    ).not.toBeInTheDocument();

    // Match the row-menu helper: open the Base UI menu with a single click event.
    fireEvent.click(screen.getByRole("button", { name: "新建或导入工作流" }));
    await user.click(
      await screen.findByRole("menuitem", { name: /新建工作流/ }),
    );
    const createDialog = await screen.findByRole("alertdialog", {
      name: "新建工作流",
    });
    await user.type(
      within(createDialog).getByLabelText("工作流名称"),
      "发布复盘",
    );
    await user.click(
      within(createDialog).getByRole("button", { name: "新建工作流" }),
    );

    await waitFor(() => {
      expect(screen.getByDisplayValue("发布复盘")).toBeInTheDocument();
      expect(
        screen.getByRole("button", { name: "发布复盘" }),
      ).toBeInTheDocument();
    });
    expect(screen.getByPlaceholderText("搜索工作流")).toHaveValue("");
    expect(screen.getByText("错开并行演示")).toBeInTheDocument();
  });

  it("keeps the create dialog open when creating a workflow fails", async () => {
    const user = userEvent.setup();
    renderEditor(undefined, createFixtureState(), (handlers) => {
      handlers.createWorkflow = async () => {
        throw new Error("disk full");
      };
    });

    await screen.findByText("代码审查工作流");
    // Match the row-menu helper: open the Base UI menu with a single click event.
    fireEvent.click(screen.getByRole("button", { name: "新建或导入工作流" }));
    await user.click(
      await screen.findByRole("menuitem", { name: /新建工作流/ }),
    );
    const createDialog = await screen.findByRole("alertdialog", {
      name: "新建工作流",
    });
    await user.type(
      within(createDialog).getByLabelText("工作流名称"),
      "发布复盘",
    );
    await user.click(
      within(createDialog).getByRole("button", { name: "新建工作流" }),
    );

    expect(
      await screen.findByRole("alertdialog", { name: "新建工作流" }),
    ).toBeInTheDocument();
    expect(await within(createDialog).findByRole("alert")).toBeInTheDocument();
    expect(within(createDialog).getByLabelText("工作流名称")).toHaveValue(
      "发布复盘",
    );
  });

  it("opens the new-workflow dialog from Ctrl+N instead of starting a chat", async () => {
    renderEditor();
    await screen.findByLabelText("工作流画布");

    await waitFor(() => {
      expect(useWorkflowEditorStore.getState().actions).not.toBeNull();
    });
    fireEvent.keyDown(window, { key: "n", ctrlKey: true });

    expect(
      await screen.findByRole("alertdialog", { name: "新建工作流" }),
    ).toBeInTheDocument();
  });

  it("describes delete as a permanent workflow deletion", async () => {
    const user = userEvent.setup();
    renderEditor();
    await screen.findByText("代码审查工作流");
    openWorkflowActions("代码审查工作流");
    await user.click(screen.getByRole("menuitem", { name: "删除" }));
    const deleteDialog = await screen.findByRole("alertdialog", {
      name: "删除“代码审查工作流”？",
    });
    expect(deleteDialog).toHaveTextContent("永久删除");
    expect(deleteDialog).not.toHaveTextContent("mock");
  });

  it("copies the current draft, opens the copy, and chains the localized suffix", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    renderEditor(undefined, state);

    await screen.findByDisplayValue("代码审查工作流");
    const source = state.workflows.find(
      (record) => record.workflow.id === "code-review",
    );
    expect(source).toBeDefined();

    openWorkflowActions("代码审查工作流");
    await user.click(screen.getByRole("menuitem", { name: "复制" }));

    await waitFor(() => {
      expect(
        screen.getByDisplayValue("代码审查工作流 - 副本"),
      ).toBeInTheDocument();
    });
    const firstCopy = state.workflows.find(
      (record) => record.workflow.name === "代码审查工作流 - 副本",
    );
    expect(firstCopy).toBeDefined();
    expect(firstCopy?.workflow.id).not.toBe(source?.workflow.id);
    expect(firstCopy?.draft.graph).toBe(source?.draft.graph);
    expect(firstCopy?.published).toEqual([]);

    openWorkflowActions("代码审查工作流 - 副本");
    await user.click(screen.getByRole("menuitem", { name: "复制" }));

    await waitFor(() => {
      expect(
        screen.getByDisplayValue("代码审查工作流 - 副本 - 副本"),
      ).toBeInTheDocument();
    });
  });

  it.each([
    ["Agent", "agent"],
    ["条件分支", "condition"],
    ["输出", "output"],
    ["退出循环", "loopExit"],
  ])(
    "adds and persists a loop %s through its member output menu",
    async (label, kind) => {
      const state = createFixtureState();
      const user = userEvent.setup();
      const view = renderEditor(<WorkflowEditor />, state);
      await screen.findByLabelText("工作流画布");
      await user.click(screen.getByRole("button", { name: "循环" }));
      expect(screen.queryByRole("button", { name: "退出循环" })).toBeNull();
      const port = await screen.findByLabelText("从循环 Agent开始连接");
      await user.hover(port);
      expect(screen.getByLabelText("在 循环 Agent 后添加节点")).toHaveClass(
        "opacity-100",
      );
      await user.click(port);
      expect(
        (await screen.findAllByRole("menuitem")).map(
          (item) => item.textContent,
        ),
      ).toEqual(["Agent", "条件分支", "输出", "退出循环"]);
      await user.click(screen.getByRole("menuitem", { name: label }));
      await waitFor(
        () => {
          const graph = JSON.parse(state.workflows[0]!.draft.graph) as {
            nodes: WorkflowDefinitionNode[];
            edges: WorkflowDefinitionEdge[];
          };
          const added = graph.nodes.find((node) => node.id === `${kind}-1`)!;
          expect(added).toMatchObject({
            data: { kind, containerId: "loop-1" },
          });
          expect(graph.edges).toEqual(
            expect.arrayContaining([
              expect.objectContaining({
                source: "loop-1-agent",
                target: added.id,
              }),
            ]),
          );
          const loop = graph.nodes.find((node) => node.id === "loop-1")!;
          expect(loop.initialWidth).toBeGreaterThan(
            added.position.x +
              (kind === "condition" ? 320 : WORKFLOW_NODE_WIDTH),
          );
        },
        { timeout: 3_000 },
      );
      if (kind === "loopExit") {
        expect(screen.queryByLabelText("从退出循环 1开始连接")).toBeNull();
      }
      if (kind === "condition") {
        await user.click(screen.getByLabelText("从条件分支 1开始连接 · else"));
        await user.click(
          await screen.findByRole("menuitem", { name: "Agent" }),
        );
        await waitFor(
          () => {
            const graph = JSON.parse(state.workflows[0]!.draft.graph);
            expect(graph.edges).toEqual(
              expect.arrayContaining([
                expect.objectContaining({
                  source: "condition-1",
                  sourceHandle: "else",
                  target: "agent-1",
                }),
              ]),
            );
          },
          { timeout: 3_000 },
        );
        await user.click(screen.getByRole("button", { name: "撤销" }));
        await waitFor(() =>
          expect(
            document.querySelector('[data-workflow-node-id="agent-1"]'),
          ).toBeNull(),
        );
        await user.click(screen.getByRole("button", { name: "重做" }));
        await waitFor(() =>
          expect(
            document.querySelector('[data-workflow-node-id="agent-1"]'),
          ).not.toBeNull(),
        );
      }
      view.unmount();
      renderEditor(<WorkflowEditor />, state, undefined, false);
      await waitFor(() =>
        expect(
          document.querySelector(`[data-workflow-node-id="${kind}-1"]`),
        ).not.toBeNull(),
      );
    },
  );

  it("persists loop end conditions through draft saving and reopening", async () => {
    const state = createFixtureState();
    const user = userEvent.setup();
    const view = renderEditor(<WorkflowEditor />, state);
    await screen.findByLabelText("工作流画布");
    await user.click(screen.getByRole("button", { name: "循环" }));
    await screen.findByLabelText("循环节点: 循环 1");
    await waitFor(
      () => {
        expect(
          JSON.parse(state.workflows[0]!.draft.graph).nodes.some(
            (node: { id: string }) => node.id === "loop-1",
          ),
        ).toBe(true);
      },
      { timeout: 3_000 },
    );
    act(() => screen.getByLabelText("条件 1").focus());
    await user.keyboard("{ArrowDown}");
    await user.click(await screen.findByRole("option", { name: "包含" }));
    await waitFor(() =>
      expect(screen.queryByRole("listbox")).not.toBeInTheDocument(),
    );
    act(() => screen.getByLabelText("值 1").focus());
    await user.keyboard("APPROVED");
    expect(screen.getByLabelText("值 1")).toHaveValue("APPROVED");
    const expected = {
      logic: "and",
      conditions: [
        {
          variableSelector: ["loop-1-agent", "output"],
          operator: "contains",
          value: "APPROVED",
        },
      ],
    };
    await waitFor(
      () => {
        const graph = JSON.parse(state.workflows[0]!.draft.graph);
        expect(
          graph.nodes.find((node: { id: string }) => node.id === "loop-1").data
            .loopConfig.until,
        ).toEqual(expected);
      },
      { timeout: 3_000 },
    );
    view.unmount();
    renderEditor(<WorkflowEditor />, state, undefined, false);
    const loop = await screen.findByLabelText("循环节点: 循环 1");
    await user.click(loop.closest(".react-flow__node") ?? loop);
    expect(await screen.findByLabelText("值 1")).toHaveValue("APPROVED");
    expect(screen.getByLabelText("条件 1")).toHaveTextContent("包含");
  });

  it("auto-saves draft edits after the debounce window", async () => {
    const state = createFixtureState();
    renderEditor(<WorkflowEditor />, state);
    const nameInput = await screen.findByLabelText("工作流名称");
    const openId = state.workflows[0]?.workflow.id;
    expect(openId).toBeDefined();

    await waitFor(() => {
      expect(screen.getByText(/已实时保存 最近修改时间：/)).toBeInTheDocument();
    });

    fireEvent.change(nameInput, { target: { value: "自动保存草稿" } });

    await waitFor(
      () => {
        const record = state.workflows.find(
          (item) => item.workflow.id === openId,
        );
        expect(record?.workflow.name).toBe("自动保存草稿");
        expect(
          screen.getByText(/已实时保存 最近修改时间：/),
        ).toBeInTheDocument();
      },
      { timeout: 3_000 },
    );
  });

  it("keeps edits only for the mounted demo session", async () => {
    const view = renderEditor();
    const nameInput = await screen.findByLabelText("工作流名称");

    fireEvent.change(nameInput, { target: { value: "当前会话草稿" } });
    expect(screen.getByDisplayValue("当前会话草稿")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "保存" }),
    ).not.toBeInTheDocument();

    view.unmount();
    renderEditor();
    expect(
      await screen.findByDisplayValue("代码审查工作流"),
    ).toBeInTheDocument();
  });

  it("preserves the current draft when the display language changes", async () => {
    renderEditor();
    const nameInput = await screen.findByLabelText("工作流名称");

    fireEvent.change(nameInput, { target: { value: "保留这个草稿" } });
    await act(() => appI18n.changeLanguage("en-US"));

    expect(screen.getByDisplayValue("保留这个草稿")).toBeInTheDocument();
    expect(screen.getByLabelText("Workflow canvas")).toBeInTheDocument();
  });

  it("localizes workflow chrome and mock content in English", async () => {
    await appI18n.changeLanguage("en-US");
    renderEditor();

    expect(await screen.findByText("Code review workflow")).toBeInTheDocument();
    expect(await screen.findByLabelText("Workflow canvas")).toBeInTheDocument();
    expect(
      screen.getByText(
        "Pointer mode: left-drag to box-select · Hand mode: left-drag to pan · Middle-drag to pan · Scroll to zoom · Nodes snap to grid",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Deploy to project" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Test run" }),
    ).not.toBeInTheDocument();
  });

  it("uses the English copy suffix when copying in English", async () => {
    const user = userEvent.setup();
    await appI18n.changeLanguage("en-US");
    renderEditor();

    await screen.findByDisplayValue("Code review workflow");
    fireEvent.click(
      screen.getByRole("button", {
        name: "Open actions for Code review workflow",
      }),
    );
    await user.click(screen.getByRole("menuitem", { name: "Copy" }));

    await waitFor(() => {
      expect(
        screen.getByDisplayValue("Code review workflow - copy"),
      ).toBeInTheDocument();
    });
  });

  it("deleting the selected workflow auto-selects the next one and loads its canvas", async () => {
    const user = userEvent.setup();
    renderEditor();

    // The mock library is seeded with code-review first and auto-selected.
    await screen.findByText("代码审查工作流");
    await screen.findByLabelText("工作流画布");
    expect(screen.getByDisplayValue("代码审查工作流")).toBeInTheDocument();

    // Delete the currently selected workflow.
    openWorkflowActions("代码审查工作流");
    await user.click(screen.getByRole("menuitem", { name: "删除" }));
    const deleteDialog = await screen.findByRole("alertdialog", {
      name: "删除“代码审查工作流”？",
    });
    await user.click(
      within(deleteDialog).getByRole("button", { name: "删除" }),
    );

    // The deleted workflow leaves the list and the first remaining one becomes selected,
    // without a "workflow not found" error or a stale canvas from the deleted workflow.
    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: "打开代码审查工作流的操作菜单" }),
      ).not.toBeInTheDocument();
    });
    expect(screen.getByText("错开并行演示")).toBeInTheDocument();
    expect(screen.getByDisplayValue("错开并行演示")).toBeInTheDocument();
    expect(screen.queryByText("未找到该工作流。")).not.toBeInTheDocument();
  });

  it("clears a leftover manager error when Back leaves the editor", async () => {
    const user = userEvent.setup();
    useUiStore.setState({ sidebarCollapsed: true, workflowEditorOpen: true });
    useWorkflowEditorStore.setState({ managerError: "stale import error" });
    renderEditor();

    await screen.findByLabelText("工作流名称");
    await user.click(screen.getByRole("button", { name: /返回|Back/ }));

    await waitFor(() => {
      expect(useUiStore.getState().workflowEditorOpen).toBe(false);
    });
    expect(useWorkflowEditorStore.getState().managerError).toBeNull();
  });

  it("keeps the editor open and reports when leaving cannot flush the draft", async () => {
    const user = userEvent.setup();
    useUiStore.setState({ sidebarCollapsed: true, workflowEditorOpen: true });
    renderEditor(undefined, createFixtureState(), (handlers) => {
      handlers.updateDraft = async () => {
        throw new Error("disk full");
      };
    });

    await screen.findByLabelText("工作流名称");
    await user.click(screen.getByRole("button", { name: /返回|Back/ }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(useUiStore.getState().workflowEditorOpen).toBe(true);
  });

  it("shows the empty-library action only after the library loads with no workflows", async () => {
    renderEditor(undefined, createFixtureState(), undefined, false);

    expect(screen.queryAllByText("还没有工作流")).toHaveLength(0);
    // Both the main pane and the sidebar list offer first-run actions.
    expect(await screen.findAllByText("还没有工作流")).toHaveLength(2);
    expect(screen.queryByLabelText("工作流画布")).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "导入" })).toBeInTheDocument();
  });

  it("shows a retryable error when the workflow library fails to load", async () => {
    const user = userEvent.setup();
    renderEditor(undefined, createFixtureState(), (handlers) => {
      const list = handlers.listWorkflows!;
      let failed = false;
      handlers.listWorkflows = async (request) => {
        if (!failed) {
          failed = true;
          throw new Error("unavailable");
        }
        return list(request);
      };
    });

    expect(await screen.findByText("无法加载工作流。")).toBeInTheDocument();
    expect(screen.queryByLabelText("工作流画布")).not.toBeInTheDocument();
    expect(screen.queryByText("还没有工作流")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "重试" }));

    expect(await screen.findByLabelText("工作流画布")).toBeInTheDocument();
  });

  it("shows a retryable error when the selected draft fails to load", async () => {
    renderEditor(undefined, createFixtureState(), (handlers) => {
      handlers.getWorkflow = async () => {
        throw new Error("unavailable");
      };
    });

    expect(await screen.findByText("无法加载工作流。")).toBeInTheDocument();
    expect(screen.queryByLabelText("工作流画布")).not.toBeInTheDocument();
  });

  it("surfaces manager errors in the editor chrome when the sidebar is collapsed", async () => {
    useUiStore.setState({ sidebarCollapsed: true, workflowEditorOpen: true });
    useWorkflowEditorStore.setState({ managerError: "disk full" });
    renderEditor();

    expect(await screen.findByRole("alert")).toHaveTextContent("disk full");
  });

  it("previews imported plugin dependencies before creating and publishing", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    renderEditor(undefined, state);
    await screen.findByLabelText("工作流画布");

    const imported = createMockWorkflow("zh-CN");
    imported.name = "导入的审查";
    const agent = imported.nodes.find((node) => node.data.kind === "agent");
    if (agent?.data.agentConfig === undefined) {
      throw new Error("fixture must contain an Agent node");
    }
    agent.data.agentConfig = {
      ...agent.data.agentConfig,
      promptInactivity: "wait",
      mcps: [{ mcpId: "acme/missing-mcp", enabled: true }],
      skills: [{ skillId: "openspec-explore", enabled: true }],
    };

    // Match the row-menu helper: open the Base UI menu with a single click event.
    fireEvent.click(screen.getByRole("button", { name: "新建或导入工作流" }));
    await user.click(
      await screen.findByRole("menuitem", { name: "导入工作流…" }),
    );
    const pickDialog = await screen.findByRole("dialog");
    expect(
      within(pickDialog).getByText("支持 .json / .reactflow.json"),
    ).toBeInTheDocument();
    await user.upload(
      within(pickDialog).getByLabelText("拖入文件，或点击选择"),
      new File([JSON.stringify(imported)], "导入的审查.v9.reactflow.json", {
        type: "application/json",
      }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText("确认导入")).toBeInTheDocument();
    expect(
      within(dialog).getAllByText("acme/missing-mcp").length,
    ).toBeGreaterThan(0);
    expect(within(dialog).getAllByText("未安装").length).toBeGreaterThan(0);
    expect(within(dialog).getAllByText("已安装").length).toBeGreaterThan(0);
    expect(within(dialog).getAllByText("去安装").length).toBeGreaterThan(0);
    expect(within(dialog).getByDisplayValue("v9")).toBeInTheDocument();

    // A missing dependency deep-links to the marketplace, searching for its identity.
    const missingRow = within(dialog)
      .getAllByText("acme/missing-mcp")[0]
      .closest("li");
    if (missingRow === null) {
      throw new Error("missing dependency row not rendered");
    }
    await user.click(
      within(missingRow).getByRole("button", { name: "去安装" }),
    );
    expect(useUiStore.getState()).toMatchObject({
      settingsOpen: true,
      settingsCategory: "plugins",
      pluginSettingsRequest: {
        kind: "marketplaceSearch",
        query: "acme/missing-mcp",
      },
    });
    act(() =>
      useUiStore.setState({
        settingsOpen: false,
        pluginSettingsRequest: null,
      }),
    );
    // Nothing is persisted while the preview is open.
    expect(
      state.workflows.some((record) => record.workflow.name === "导入的审查"),
    ).toBe(false);

    await user.click(within(dialog).getByRole("button", { name: "仍然导入" }));

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
    await waitFor(() => {
      const record = state.workflows.find(
        (item) => item.workflow.name === "导入的审查",
      );
      expect(record?.published.map((snapshot) => snapshot.version)).toEqual([
        "v9",
      ]);
      expect(
        parseWorkflowGraph(record!.published[0]!.graph).nodes.find(
          (node) => node.id === agent.id,
        )?.data.agentConfig,
      ).toEqual(agent.data.agentConfig);
    });
    expect(await screen.findByDisplayValue("导入的审查")).toBeInTheDocument();
    expect(screen.getByText("新导入")).toBeInTheDocument();
  });

  it("previews a workflow dropped onto the import picker", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    renderEditor(undefined, state);
    await screen.findByLabelText("工作流画布");

    const imported = createMockWorkflow("zh-CN");
    imported.name = "拖入的工作流";
    fireEvent.click(screen.getByRole("button", { name: "新建或导入工作流" }));
    await user.click(
      await screen.findByRole("menuitem", { name: "导入工作流…" }),
    );
    const pickDialog = await screen.findByRole("dialog");
    const dropZone =
      within(pickDialog).getByLabelText("拖入文件，或点击选择").parentElement;
    if (dropZone === null) {
      throw new Error("import drop zone is missing");
    }
    const file = new File(
      [JSON.stringify(imported)],
      "拖入的工作流.reactflow.json",
      { type: "application/json" },
    );
    fireEvent.drop(dropZone, {
      dataTransfer: {
        files: [file],
        items: [],
        types: ["Files"],
        dropEffect: "copy",
      },
    });

    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByText("确认导入")).toBeInTheDocument();
    expect(
      within(dialog).getByText(/拖入的工作流\.reactflow\.json/),
    ).toBeInTheDocument();
  });

  it("explains an unreadable import file without creating a workflow", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    renderEditor(undefined, state);
    await screen.findByLabelText("工作流画布");
    const workflowCount = state.workflows.length;

    // Match the row-menu helper: open the Base UI menu with a single click event.
    fireEvent.click(screen.getByRole("button", { name: "新建或导入工作流" }));
    await user.click(
      await screen.findByRole("menuitem", { name: "导入工作流…" }),
    );
    const pickDialog = await screen.findByRole("dialog");
    await user.upload(
      within(pickDialog).getByLabelText("拖入文件，或点击选择"),
      new File(["{ nope"], "broken.json", { type: "application/json" }),
    );

    const dialog = await screen.findByRole("dialog");
    expect(await within(dialog).findByRole("alert")).toHaveTextContent(
      "文件不是有效的 JSON。",
    );
    expect(within(dialog).getByText("无法导入")).toBeInTheDocument();
    await user.click(within(dialog).getByRole("button", { name: "关闭" }));

    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
    expect(state.workflows).toHaveLength(workflowCount);
  });

  it("exports the draft or a chosen published version", async () => {
    const user = userEvent.setup();
    renderEditor();
    await screen.findByLabelText("工作流画布");

    await user.click(screen.getByRole("button", { name: "导出" }));

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByRole("radio", { name: /当前草稿/ }),
    ).toHaveAttribute("aria-checked", "true");
    expect(
      within(dialog).getByDisplayValue("代码审查工作流.reactflow.json"),
    ).toBeInTheDocument();
    expect(
      within(dialog).getByText("随文件记录的插件引用"),
    ).toBeInTheDocument();
    expect(within(dialog).getByText("预览文件结构")).toBeInTheDocument();

    const [, publishedVersion] = within(dialog).getAllByRole("radio");
    await user.click(publishedVersion);

    expect(publishedVersion).toHaveAttribute("aria-checked", "true");
    expect(
      await within(dialog).findByText("随文件记录的插件引用"),
    ).toBeInTheDocument();
    expect(
      within(dialog).queryByDisplayValue("代码审查工作流.reactflow.json"),
    ).not.toBeInTheDocument();

    await user.click(within(dialog).getByRole("button", { name: "取消" }));
    await waitFor(() => {
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
  });
});

describe("useCreateWorkflow", () => {
  it("prepends the created workflow onto the library cache synchronously", async () => {
    const state = createFixtureState();
    seedDemoWorkflows(state);
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const { result, queryClient } = renderHookWithClient(
      () => useCreateWorkflow(),
      client,
    );
    await queryClient.fetchQuery({
      queryKey: ["workflow", "library"],
      queryFn: async () => (await client.workflow.list({})).workflows,
    });
    const before = queryClient.getQueryData(["workflow", "library"]) as Array<{
      id: string;
    }>;
    expect(before[0]?.id).toBe("code-review");

    await act(async () => {
      await result.current.mutateAsync({ name: "发布复盘" });
    });

    const after = queryClient.getQueryData(["workflow", "library"]) as Array<{
      id: string;
      name: string;
    }>;
    expect(after[0]?.name).toBe("发布复盘");
    expect(after[0]?.id).not.toBe("code-review");
  });
});

describe("useDeleteWorkflow", () => {
  it("removes the deleted workflow from the library cache synchronously", async () => {
    const state = createFixtureState();
    seedDemoWorkflows(state);
    const clientHandlers: TestHandlers = createFixtureHandlers(state);
    const client = createTestClient(clientHandlers);
    const { result, queryClient } = renderHookWithClient(
      () => useDeleteWorkflow(),
      client,
    );
    // Pre-warm the library query like the editor does.
    await queryClient.fetchQuery({
      queryKey: ["workflow", "library"],
      queryFn: async () => (await client.workflow.list({})).workflows,
    });
    const before = queryClient.getQueryData(["workflow", "library"]) as Array<{
      id: string;
    }>;
    expect(before.some((item) => item.id === "code-review")).toBe(true);

    await act(async () => {
      await result.current.mutateAsync("code-review");
    });

    // The cache must drop the row immediately, before any invalidateQueries refetch lands,
    // so the editor auto-select reads a list that no longer contains the deleted id.
    const after = queryClient.getQueryData(["workflow", "library"]) as Array<{
      id: string;
    }>;
    expect(after.some((item) => item.id === "code-review")).toBe(false);
  });

  it("resizes an iteration frame from its corner as one undoable edit", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    seedDemoWorkflows(state);
    seedIterationShowcase(state);
    useWorkflowEditorStore.setState({
      selectedWorkflowId: "iteration-showcase",
    });
    const restoreOffsetSize = stubNodeWrapperOffsetSize();
    renderEditor(undefined, state, undefined, false);

    try {
      expect(
        await screen.findByLabelText("评审迭代: 迭代"),
      ).toBeInTheDocument();
      const frame = () =>
        document.querySelector<HTMLElement>("[data-workflow-iteration-frame]")!;
      expect(frame()).toHaveStyle({
        width: `${WORKFLOW_ITERATION_NODE_WIDTH}px`,
        height: `${WORKFLOW_ITERATION_NODE_HEIGHT}px`,
      });
      const control = document.querySelector<HTMLElement>(
        ".react-flow__resize-control.handle.bottom.right",
      );
      expect(control).not.toBeNull();
      control!.setPointerCapture = () => {};

      await act(async () => {
        control!.dispatchEvent(
          windowedMouseEvent("mousedown", {
            button: 0,
            clientX: 600,
            clientY: 380,
          }),
        );
      });
      await act(async () => {
        window.dispatchEvent(
          windowedMouseEvent("mousemove", {
            button: 0,
            clientX: 700,
            clientY: 460,
          }),
        );
      });
      await act(async () => {
        window.dispatchEvent(
          windowedMouseEvent("mouseup", {
            button: 0,
            clientX: 700,
            clientY: 460,
          }),
        );
      });

      // The corner drag enlarged the frame and persisted the authored size.
      expect(frame()).toHaveStyle({
        width: `${WORKFLOW_ITERATION_NODE_WIDTH + 100}px`,
        height: `${WORKFLOW_ITERATION_NODE_HEIGHT + 80}px`,
      });

      // The resize is one semantic history step with a localized label.
      await user.click(screen.getByRole("button", { name: "变更历史" }));
      expect(
        await screen.findByText(/调整迭代区域大小：评审迭代/),
      ).toBeInTheDocument();

      // Undo restores the previous frame size in one step.
      await user.click(screen.getByRole("button", { name: "撤销" }));
      await waitFor(() => {
        expect(frame()).toHaveStyle({
          width: `${WORKFLOW_ITERATION_NODE_WIDTH}px`,
          height: `${WORKFLOW_ITERATION_NODE_HEIGHT}px`,
        });
      });
    } finally {
      restoreOffsetSize();
    }
  }, 20_000);

  it("adds an iteration member end-to-end by clicking the internal start port", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    seedDemoWorkflows(state);
    seedIterationShowcase(state);
    useWorkflowEditorStore.setState({
      selectedWorkflowId: "iteration-showcase",
    });
    renderEditor(undefined, state, undefined, false);

    expect(await screen.findByLabelText("评审迭代: 迭代")).toBeInTheDocument();
    // The entry port doubles as the picker trigger (Dify behavior), while the
    // hover plus stays decorative so port drags keep starting connections.
    await user.click(screen.getByLabelText("连接到循环体首节点"));
    await user.click(await screen.findByRole("menuitem", { name: "Agent" }));

    // The insert lands in the draft graph: one new member card selected on canvas.
    await waitFor(() => {
      expect(
        document.querySelector('[data-workflow-node-id="agent-1"]'),
      ).not.toBeNull();
    });
    const inserted = document.querySelector<HTMLElement>(
      '[data-workflow-node-id="agent-1"]',
    )!;
    expect(inserted).toHaveTextContent("Agent 1");
    expect(
      document.querySelector("[data-workflow-node-count]"),
    ).toHaveAttribute("data-workflow-node-count", "4");
  }, 20_000);

  it("appends an iteration member end-to-end from a member output port", async () => {
    const user = userEvent.setup();
    const state = createFixtureState();
    seedDemoWorkflows(state);
    seedIterationShowcase(state);
    useWorkflowEditorStore.setState({
      selectedWorkflowId: "iteration-showcase",
    });
    renderEditor(undefined, state, undefined, false);

    // The unoccupied member output owns the append affordance.
    await user.click(await screen.findByLabelText("从评审成员开始连接"));
    await user.click(await screen.findByRole("menuitem", { name: "条件分支" }));

    await waitFor(() => {
      expect(
        document.querySelector('[data-workflow-node-id="condition-1"]'),
      ).not.toBeNull();
    });
    expect(
      document.querySelector("[data-workflow-node-count]"),
    ).toHaveAttribute("data-workflow-node-count", "4");
  }, 20_000);
});
