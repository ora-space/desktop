import { act, fireEvent, render, screen, waitFor, within, type RenderResult } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ReactElement } from "react";
import { createChatStore } from "@ora/chat";
import { PlatformProvider } from "@ora/platform";
import { appI18n } from "../../i18n/i18n-instance";
import { AppI18nProvider } from "../../i18n/i18n";
import { createHookWrapper, createTestQueryClient } from "../../test/hook-harness";
import { createMockClient, createMockClientState } from "../../test/mock-client";
import { createStubPlatform } from "../../test/stub-platform";
import { WorkflowSettings } from "./workflow-settings";

/** Shell providers required by Deploy-to-project (runtime + react-query). */
function renderSettings(ui: ReactElement = <WorkflowSettings />): RenderResult {
  const client = createMockClient(createMockClientState());
  const Wrapper = createHookWrapper(
    client,
    createTestQueryClient(),
    createChatStore(client.session),
  );
  return render(
    <Wrapper>
      <PlatformProvider adapter={createStubPlatform()}>
        <AppI18nProvider>{ui}</AppI18nProvider>
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

describe("WorkflowSettings", () => {
  beforeEach(() => {
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
    await appI18n.changeLanguage("zh-CN");
  });

  it("loads the mock graph and deploy control without an in-settings test run", async () => {
    renderSettings();

    expect(await screen.findByText("代码审查工作流")).toBeInTheDocument();
    expect(screen.getByLabelText("工作流画布")).toBeInTheDocument();
    expect(screen.getByRole("separator", {
      name: "调整工作流列表宽度；双击恢复默认宽度",
    })).toBeInTheDocument();
    expect(screen.getByRole("separator", {
      name: "调整节点配置宽度；双击恢复默认宽度",
    })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "部署到项目" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "导出工作流" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "测试运行" })).not.toBeInTheDocument();
  });

  it("zooms around the pointer with the mouse wheel", async () => {
    renderSettings();
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
    renderSettings();
    await screen.findByLabelText("工作流画布");
    const viewport = flowViewport();

    expect(viewport?.style.transform).toContain("translate(32px,32px)");
    expect(screen.getByText("100%")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "放大画布" }));
    await waitFor(() => {
      expect(screen.queryByText("100%")).not.toBeInTheDocument();
    });

    expect(screen.getByRole("button", { name: "显示完整工作流" })).toBeInTheDocument();
    expect(screen.getByLabelText("工作流小地图")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "重置画布视图" }));
    await waitFor(() => {
      expect(screen.getByText("100%")).toBeInTheDocument();
      expect(viewport?.style.transform).toContain("translate(32px,32px)");
    });
  });

  it("does not pan from the panel-resize guard zones at canvas edges", async () => {
    renderSettings();
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
    renderSettings();
    await screen.findByLabelText("开始节点: 开始");

    expect(nodeGraphPosition("开始节点: 开始")).toEqual({
      x: "72px",
      y: "286px",
    });
    expect(nodeGraphPosition("提示词节点: 理解改动")).toEqual({
      x: "356px",
      y: "188px",
    });
  });

  it("keeps each workflow port independently visible without node-wide hover styles", async () => {
    renderSettings();
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
    renderSettings();
    const startNode = await screen.findByLabelText("开始节点: 开始");
    const flowNode = startNode.closest(".react-flow__node") ?? startNode;

    await user.click(flowNode);
    expect(screen.getByRole("button", { name: "收起节点配置" })).toBeInTheDocument();

    const pane = document.querySelector(".react-flow__pane");
    expect(pane).not.toBeNull();
    fireEvent.click(pane!);

    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "收起节点配置" })).not.toBeInTheDocument();
    });
  });

  it("collapses and restores the workflow library from visible controls", async () => {
    const user = userEvent.setup();
    renderSettings();
    await screen.findByText("代码审查工作流");

    await user.click(screen.getByRole("button", { name: "收起工作流列表" }));
    const expandButton = await screen.findByRole("button", { name: "展开工作流列表" });
    await user.click(expandButton);

    expect(screen.getByRole("button", { name: "收起工作流列表" })).toBeInTheDocument();
  });

  it("keeps only one auxiliary panel expanded in a narrow editor", async () => {
    const user = userEvent.setup();
    renderSettings();
    const startNode = await screen.findByLabelText("开始节点: 开始");
    const flowNode = startNode.closest(".react-flow__node") ?? startNode;

    await user.click(flowNode);
    await user.click(screen.getByRole("button", { name: "展开工作流列表" }));

    await waitFor(() => {
      expect(screen.getByRole("button", { name: "展开节点配置" })).toBeInTheDocument();
    });
  });

  it("closes node configuration with its button or Escape", async () => {
    const user = userEvent.setup();
    renderSettings();
    const startNode = await screen.findByLabelText("开始节点: 开始");
    const flowNode = startNode.closest(".react-flow__node") ?? startNode;

    await user.click(flowNode);
    await user.click(screen.getByRole("button", { name: "收起节点配置" }));
    expect(screen.queryByRole("button", { name: "收起节点配置" })).not.toBeInTheDocument();

    await user.click(flowNode);
    fireEvent.keyDown(startNode, { key: "Escape" });

    await waitFor(() => {
      expect(screen.queryByRole("button", { name: "收起节点配置" })).not.toBeInTheDocument();
    });
  });

  it("switches workflows from the manager and adds nodes from the bottom dock", async () => {
    const user = userEvent.setup();
    renderSettings();

    const releaseWorkflow = await screen.findByText("发布准备检查");
    await user.click(releaseWorkflow.closest("button")!);

    expect(screen.getByDisplayValue("发布准备检查")).toBeInTheDocument();
    expect(screen.getByLabelText("添加工作流节点")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "开始" })).not.toBeInTheDocument();
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

    await user.click(screen.getByRole("button", { name: "提示词" }));

    expect(screen.getAllByText("提示词 1")).toHaveLength(2);
    expect(nodeGraphPosition("提示词节点: 提示词 1")).toEqual({
      x: "260px",
      y: "200px",
    });
  });

  it("drags a node type from the dock to the chosen canvas position", async () => {
    renderSettings();
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
    const toolButton = screen.getByRole("button", { name: "工具" });
    toolButton.setPointerCapture = () => {};

    expect(canvas).not.toContainElement(toolButton);
    fireEvent.pointerDown(toolButton, {
      button: 0,
      isPrimary: true,
      pointerId: 1,
      clientX: 400,
      clientY: 650,
    });
    fireEvent.pointerMove(toolButton, {
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

    fireEvent.pointerUp(toolButton, {
      isPrimary: true,
      pointerId: 1,
      clientX: 500,
      clientY: 350,
    });
    fireEvent.click(toolButton);

    expect(document.querySelector("[data-workflow-node-preview]")).not.toBeInTheDocument();
    expect(nodeGraphPosition("工具节点: 工具 1")).toEqual({
      x: "360px",
      y: "260px",
    });
    expect(screen.queryByText("释放以添加节点")).not.toBeInTheDocument();
  });

  it("deletes workflow connections by double-click or keyboard", async () => {
    const user = userEvent.setup();
    renderSettings();

    const connection = await screen.findByRole("button", {
      name: "Edge from start to understand",
    });
    await user.dblClick(connection);

    await waitFor(() => {
      expect(screen.queryByRole("button", {
        name: "Edge from start to understand",
      })).not.toBeInTheDocument();
    });

    const keyboardConnection = screen.getByRole("button", {
      name: "Edge from understand to quality",
    });
    await user.click(keyboardConnection);
    await user.keyboard("{Delete}");

    await waitFor(() => {
      expect(screen.queryByRole("button", {
        name: "Edge from understand to quality",
      })).not.toBeInTheDocument();
    });
  });

  it("restores each workflow from its React Flow viewport snapshot", async () => {
    const user = userEvent.setup();
    renderSettings();
    await screen.findByLabelText("工作流画布");

    await user.click(screen.getByRole("button", { name: "放大画布" }));
    await waitFor(() => {
      expect(screen.queryByText("100%")).not.toBeInTheDocument();
    });
    const editedViewport = flowViewport()?.style.transform;

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
    renderSettings();

    const node = await screen.findByLabelText("提示词节点: 理解改动");
    expect(screen.getByRole("button", {
      name: "Edge from start to understand",
    })).toBeInTheDocument();
    expect(screen.getByRole("button", {
      name: "Edge from understand to quality",
    })).toBeInTheDocument();

    await user.click(node.closest(".react-flow__node") ?? node);
    await user.click(screen.getByRole("button", { name: "删除理解改动" }));

    await waitFor(() => {
      expect(screen.queryByLabelText("提示词节点: 理解改动")).not.toBeInTheDocument();
      expect(screen.queryByRole("button", {
        name: "Edge from start to understand",
      })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", {
        name: "Edge from understand to quality",
      })).not.toBeInTheDocument();
    });
  });

  it("uses React Flow deletable state to protect the required start node", async () => {
    const user = userEvent.setup();
    renderSettings();

    const startNode = await screen.findByLabelText("开始节点: 开始");
    await user.click(startNode.closest(".react-flow__node") ?? startNode);

    expect(screen.queryByRole("button", { name: "删除开始" })).not.toBeInTheDocument();
    await user.keyboard("{Delete}");
    expect(screen.getByLabelText("开始节点: 开始")).toBeInTheDocument();
  });

  it("edits the existing Agent node through its structured execution contract", async () => {
    const user = userEvent.setup();
    renderSettings();

    const reviewNode = await screen.findByLabelText("Agent节点: 审查 Agent");
    await user.click(reviewNode.closest(".react-flow__node") ?? reviewNode);

    expect(screen.getByLabelText("Agent 模型")).toBeInTheDocument();
    expect(screen.getByLabelText("角色")).toHaveTextContent("审查员");
    expect(screen.getByText("Skills")).toBeInTheDocument();
    expect(screen.getByLabelText("自定义 Prompt")).toHaveValue(
      "按严重程度整理问题，并给出定位与修复建议。",
    );
    expect(screen.queryByText("输入上下文")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("项目权限")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("输出契约")).not.toBeInTheDocument();
    await waitFor(() => {
      expect(screen.getByLabelText("Agent 模型")).toHaveTextContent(
        "OpenCode · opencode/big-pickle",
      );
    });
  });

  it("uses the backend catalog for a newly added Agent model", async () => {
    const user = userEvent.setup();
    renderSettings();

    const reviewNode = await screen.findByLabelText("Agent节点: 审查 Agent");
    await user.click(reviewNode.closest(".react-flow__node") ?? reviewNode);
    const modelSelect = screen.getByLabelText("Agent 模型");
    await waitFor(() => expect(modelSelect).toBeEnabled());
    await user.click(screen.getByRole("button", { name: "Agent" }));

    expect(await screen.findByLabelText("Agent节点: Agent 1")).toBeInTheDocument();
    expect(screen.getAllByText("open_code · opencode/big-pickle").length).toBeGreaterThan(0);
  });

  it("adds, disables, and removes configured Agent Skills", async () => {
    const user = userEvent.setup();
    renderSettings();

    const reviewNode = await screen.findByLabelText("Agent节点: 审查 Agent");
    await user.click(reviewNode.closest(".react-flow__node") ?? reviewNode);
    const existingSkillSwitch = screen.getByRole("switch", {
      name: "启用或禁用 openspec-verify-change",
    });
    expect(existingSkillSwitch).toBeChecked();
    expect(screen.queryByRole("checkbox")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "添加 Skill" }));
    const skillSearch = screen.getByLabelText("搜索可添加的 Skill");
    await user.type(skillSearch, "archive");
    await user.click(screen.getByText("openspec-archive-change"));

    const archiveSwitch = screen.getByRole("switch", {
      name: "启用或禁用 openspec-archive-change",
    });
    expect(archiveSwitch).toBeChecked();
    await user.click(archiveSwitch);
    expect(archiveSwitch).not.toBeChecked();

    await user.click(screen.getByRole("button", {
      name: "移除 openspec-archive-change",
    }));
    expect(screen.queryByText("openspec-archive-change")).not.toBeInTheDocument();
  });

  it("routes inspector deletion through the shared React Flow store", async () => {
    const user = userEvent.setup();
    renderSettings();

    const reviewNode = await screen.findByLabelText("Agent节点: 审查 Agent");
    await user.click(reviewNode.closest(".react-flow__node") ?? reviewNode);
    await user.click(screen.getByRole("button", { name: "删除节点" }));

    await waitFor(() => {
      expect(screen.queryByLabelText("Agent节点: 审查 Agent")).not.toBeInTheDocument();
      expect(screen.queryByRole("button", {
        name: "Edge from quality to review",
      })).not.toBeInTheDocument();
      expect(screen.queryByRole("button", {
        name: "Edge from review to output",
      })).not.toBeInTheDocument();
    });
  });

  it("shows React Flow reconnect controls after selecting an edge", async () => {
    const user = userEvent.setup();
    renderSettings();

    const connection = await screen.findByRole("button", {
      name: "Edge from start to understand",
    });
    await user.click(connection);

    await waitFor(() => {
      expect(document.querySelector(".react-flow__edgeupdater-source")).not.toBeNull();
      expect(document.querySelector(".react-flow__edgeupdater-target")).not.toBeNull();
    });
  });

  it("creates a workflow from the left manager and allows renaming it", async () => {
    const user = userEvent.setup();
    renderSettings();

    await screen.findByText("代码审查工作流");
    await user.click(screen.getByRole("button", { name: "新建工作流" }));
    const createDialog = await screen.findByRole("alertdialog", { name: "新建工作流" });
    const createNameInput = within(createDialog).getByLabelText("工作流名称");
    await user.type(createNameInput, "发布复盘");
    await user.click(within(createDialog).getByRole("button", { name: "新建工作流" }));

    await waitFor(() => {
      expect(screen.getByDisplayValue("发布复盘")).toBeInTheDocument();
      expect(screen.getByText("7 个工作流")).toBeInTheDocument();
    });

    await user.click(screen.getByRole("button", { name: "重命名发布复盘" }));
    const renameDialog = await screen.findByRole("alertdialog", { name: "重命名“发布复盘”" });
    const renameNameInput = within(renameDialog).getByDisplayValue("发布复盘");
    await user.clear(renameNameInput);
    await user.type(renameNameInput, "发布复盘 v2");
    await user.click(within(renameDialog).getByRole("button", { name: "重命名" }));

    await waitFor(() => {
      expect(screen.getByDisplayValue("发布复盘 v2")).toBeInTheDocument();
    });
  });

  it("keeps edits only for the mounted demo session", async () => {
    const view = renderSettings();
    const nameInput = await screen.findByLabelText("工作流名称");

    fireEvent.change(nameInput, { target: { value: "当前会话草稿" } });
    expect(screen.getByDisplayValue("当前会话草稿")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "保存" })).not.toBeInTheDocument();

    view.unmount();
    renderSettings();
    expect(await screen.findByDisplayValue("代码审查工作流")).toBeInTheDocument();
  });

  it("preserves the current draft when the display language changes", async () => {
    renderSettings();
    const nameInput = await screen.findByLabelText("工作流名称");

    fireEvent.change(nameInput, { target: { value: "保留这个草稿" } });
    await act(() => appI18n.changeLanguage("en-US"));

    expect(screen.getByDisplayValue("保留这个草稿")).toBeInTheDocument();
    expect(screen.getByLabelText("Workflow canvas")).toBeInTheDocument();
  });

  it("localizes workflow chrome and mock content in English", async () => {
    await appI18n.changeLanguage("en-US");
    renderSettings();

    expect(await screen.findByText("Code review workflow")).toBeInTheDocument();
    expect(screen.getByLabelText("Workflow canvas")).toBeInTheDocument();
    expect(
      screen.getByText("Scroll to zoom · Drag to pan · Nodes snap to grid"),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Deploy to project" })).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Test run" })).not.toBeInTheDocument();
  });
});
