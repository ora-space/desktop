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
  createMockWorkflowVersions,
  createMockWorkflows,
} from "@ora/workflow-mock";
import {
  serializeWorkflowGraph,
  type WorkflowDefinitionEdge,
  type WorkflowDefinitionNode,
} from "@ora/workflow-runtime";
import { TooltipProvider } from "@ora/ui";
import { appI18n } from "../../i18n/i18n-instance";
import { AppI18nProvider } from "../../i18n/i18n";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import {
  createMockClient,
  createMockClientState,
  type MockClientState,
} from "../../test/mock-client";
import { renderHookWithClient } from "../../test/hook-harness";
import { createStubPlatform } from "../../test/stub-platform";
import { useUiStore } from "../../state/stores/ui-store";
import { useCreateWorkflow, useDeleteWorkflow } from "./workflow-definitions";
import { WorkflowEditor } from "./workflow-editor";
import { WorkflowEditorList } from "./workflow-editor-list";
import { useWorkflowEditorStore } from "./workflow-editor-store";

/** Seeds the mock client with the demo workflows and their published versions. */
function seedDemoWorkflows(state: MockClientState): void {
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

/** Shell providers required by the workspace workflow editor (runtime + react-query). */
function renderEditor(
  ui?: ReactElement,
  state: MockClientState = createMockClientState(),
  patchClient?: (client: ReturnType<typeof createMockClient>) => void,
  seedLibrary = true,
): RenderResult {
  if (seedLibrary) {
    seedDemoWorkflows(state);
  }
  // Model discovery needs a project cwd so warmSession can report real model catalogs.
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
  // Warm-session model catalog consumed by the workflow inspector's model selector.
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
  const client = createMockClient(state);
  patchClient?.(client);
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
    expect(
      screen.getByRole("button", { name: "导出工作流" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "打开代码审查工作流的操作菜单" }),
    ).toHaveClass("opacity-100");
    expect(
      screen.queryByRole("button", { name: "测试运行" }),
    ).not.toBeInTheDocument();
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

    // The card carries the title text and the inspector header exposes the
    // same title as an editable input (Dify-style grouped layout).
    expect(screen.getByLabelText("Agent节点: Agent 1")).toHaveTextContent(
      "Agent 1",
    );
    expect(screen.getByLabelText("名称")).toHaveValue("Agent 1");
    expect(nodeGraphPosition("Agent节点: Agent 1")).toEqual({
      x: "260px",
      y: "200px",
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
    await user.click(reviewNode.closest(".react-flow__node") ?? reviewNode);

    expect(screen.getByLabelText("Agent 模型")).toBeInTheDocument();
    expect(screen.getByLabelText("角色")).toHaveTextContent("Reviewer");
    expect(screen.getAllByText("Skills")).toHaveLength(2);
    expect(screen.getByLabelText("自定义 Prompt")).toHaveValue(
      "按严重程度整理问题，并给出定位与修复建议。",
    );
    expect(screen.queryByText("输入上下文")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("项目权限")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("输出契约")).not.toBeInTheDocument();
    await waitFor(() => {
      expect(screen.getByLabelText("Agent 模型")).toHaveTextContent(
        "Big Pickle",
      );
    });
    const configuredParameters = within(reviewNode).getByLabelText("配置参数");
    expect(configuredParameters).toHaveTextContent("角色Reviewer");
    expect(configuredParameters).toHaveTextContent(
      "ora-space.codeagentcli · opencode/big-pickle",
    );
    expect(configuredParameters).toHaveTextContent("Skillscode-defect-scan");
    expect(configuredParameters).not.toHaveTextContent(
      "按严重程度整理问题，并给出定位与修复建议。",
    );
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
    const state = createMockClientState();
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
    // NGA exists as a CLI but its warm session reports no model catalog, so
    // picking it must keep the node on NGA instead of snapping back to the
    // first CLI with discovered models.
    state.warmModelsByCli = { "ora-space.nga": null };
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
      screen.getAllByText("ora-space.opencode · opencode/big-pickle").length,
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
    await user.click(screen.getByLabelText("新建工作流"));
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

    await user.click(screen.getByLabelText("新建工作流"));
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
    renderEditor(undefined, createMockClientState(), (client) => {
      client.workflow.create = async () => {
        throw new Error("disk full");
      };
    });

    await screen.findByText("代码审查工作流");
    await user.click(screen.getByLabelText("新建工作流"));
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
    const state = createMockClientState();
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

  it("auto-saves draft edits after the debounce window", async () => {
    const state = createMockClientState();
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
    renderEditor(undefined, createMockClientState(), (client) => {
      client.workflow.updateDraft = async () => {
        throw new Error("disk full");
      };
    });

    await screen.findByLabelText("工作流名称");
    await user.click(screen.getByRole("button", { name: /返回|Back/ }));

    expect(await screen.findByRole("alert")).toBeInTheDocument();
    expect(useUiStore.getState().workflowEditorOpen).toBe(true);
  });

  it("shows the empty-library action only after the library loads with no workflows", async () => {
    renderEditor(undefined, createMockClientState(), undefined, false);

    expect(screen.queryByText("还没有工作流")).not.toBeInTheDocument();
    expect(await screen.findByText("还没有工作流")).toBeInTheDocument();
    expect(screen.queryByLabelText("工作流画布")).not.toBeInTheDocument();
  });

  it("shows a retryable error when the workflow library fails to load", async () => {
    const user = userEvent.setup();
    renderEditor(undefined, createMockClientState(), (client) => {
      const list = client.workflow.list;
      let failed = false;
      client.workflow.list = async (request) => {
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
    renderEditor(undefined, createMockClientState(), (client) => {
      client.workflow.get = async () => {
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
});

describe("useCreateWorkflow", () => {
  it("prepends the created workflow onto the library cache synchronously", async () => {
    const state = createMockClientState();
    seedDemoWorkflows(state);
    const client = createMockClient(state);
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
    const state = createMockClientState();
    seedDemoWorkflows(state);
    const client = createMockClient(state);
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
});
