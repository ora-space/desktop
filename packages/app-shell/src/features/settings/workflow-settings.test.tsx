import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { WorkflowSettings } from "./workflow-settings";

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

  it("loads the mock graph and exposes a deterministic preview", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);

    expect(screen.getByText("正在加载 mock 工作流…")).toBeInTheDocument();
    expect(await screen.findByText("代码审查工作流")).toBeInTheDocument();
    expect(screen.getByLabelText("工作流画布")).toBeInTheDocument();
    expect(screen.getByRole("separator", {
      name: "调整工作流列表宽度；双击恢复默认宽度",
    })).toBeInTheDocument();
    expect(screen.getByRole("separator", {
      name: "调整节点配置宽度；双击恢复默认宽度",
    })).toBeInTheDocument();

    await user.click(screen.getAllByRole("button", { name: "测试运行" })[0]);

    expect(await screen.findByText("模拟运行成功")).toBeInTheDocument();
    expect(screen.getByText(/发现 2 个建议项，未发现阻塞问题。/)).toBeInTheDocument();

    const pane = document.querySelector(".react-flow__pane");
    expect(pane).not.toBeNull();
    fireEvent.click(pane!);

    expect(screen.getByText("模拟运行成功")).toBeInTheDocument();
  });

  it("zooms around the pointer with the mouse wheel", async () => {
    render(<WorkflowSettings />);
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
    render(<WorkflowSettings />);
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
    render(<WorkflowSettings />);
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
    render(<WorkflowSettings />);
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
    render(<WorkflowSettings />);
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
    render(<WorkflowSettings />);
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
    render(<WorkflowSettings />);
    await screen.findByText("代码审查工作流");

    await user.click(screen.getByRole("button", { name: "收起工作流列表" }));
    const expandButton = await screen.findByRole("button", { name: "展开工作流列表" });
    await user.click(expandButton);

    expect(screen.getByRole("button", { name: "收起工作流列表" })).toBeInTheDocument();
  });

  it("keeps only one auxiliary panel expanded in a narrow editor", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);
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
    render(<WorkflowSettings />);
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
    render(<WorkflowSettings />);

    const releaseWorkflow = await screen.findByText("发布准备检查");
    await user.click(releaseWorkflow.closest("button")!);

    expect(screen.getByDisplayValue("发布准备检查")).toBeInTheDocument();
    expect(screen.getByLabelText("添加工作流节点")).toBeInTheDocument();
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
    render(<WorkflowSettings />);
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
    render(<WorkflowSettings />);

    const connection = await screen.findByRole("button", {
      name: "选择从开始到理解改动的连线",
    });
    await user.dblClick(connection);

    expect(screen.queryByRole("button", {
      name: "选择从开始到理解改动的连线",
    })).not.toBeInTheDocument();

    const keyboardConnection = screen.getByRole("button", {
      name: "选择从理解改动到质量门禁的连线",
    });
    await user.click(keyboardConnection);
    fireEvent.keyDown(keyboardConnection, { key: "Delete" });

    expect(screen.queryByRole("button", {
      name: "选择从理解改动到质量门禁的连线",
    })).not.toBeInTheDocument();
  });

  it("shows React Flow reconnect controls after selecting an edge", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);

    const connection = await screen.findByRole("button", {
      name: "选择从开始到理解改动的连线",
    });
    await user.click(connection);

    await waitFor(() => {
      expect(document.querySelector(".react-flow__edgeupdater-source")).not.toBeNull();
      expect(document.querySelector(".react-flow__edgeupdater-target")).not.toBeNull();
    });
  });

  it("creates a workflow from the left manager and allows renaming it", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);

    await screen.findByText("代码审查工作流");
    await user.click(screen.getByRole("button", { name: "新建工作流" }));
    const createDialog = await screen.findByRole("alertdialog", { name: "新建工作流" });
    const createNameInput = within(createDialog).getByLabelText("工作流名称");
    await user.type(createNameInput, "发布复盘");
    await user.click(within(createDialog).getByRole("button", { name: "新建工作流" }));

    await waitFor(() => {
      expect(screen.getByDisplayValue("发布复盘")).toBeInTheDocument();
      expect(screen.getByText("4 个工作流")).toBeInTheDocument();
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

  it("keeps newer draft edits when an older save finishes", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);
    const nameInput = await screen.findByLabelText("工作流名称");

    fireEvent.change(nameInput, { target: { value: "保存中的版本" } });
    await user.click(screen.getByRole("button", { name: "保存" }));
    fireEvent.change(screen.getByLabelText("工作流名称"), {
      target: { value: "保存后继续编辑" },
    });

    await waitFor(() => {
      expect(screen.getByDisplayValue("保存后继续编辑")).toBeInTheDocument();
      expect(screen.getByRole("button", { name: "保存" })).toBeEnabled();
    });
  });

  it("runs the unsaved draft visible in the editor", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);
    const nameInput = await screen.findByLabelText("工作流名称");

    fireEvent.change(nameInput, { target: { value: "未保存草稿" } });
    await user.click(screen.getAllByRole("button", { name: "测试运行" })[0]);

    expect(await screen.findByText(/已完成“未保存草稿”的模拟运行/)).toBeInTheDocument();
  });

  it("preserves the current draft when the display language changes", async () => {
    render(<WorkflowSettings />);
    const nameInput = await screen.findByLabelText("工作流名称");

    fireEvent.change(nameInput, { target: { value: "保留这个草稿" } });
    await act(() => appI18n.changeLanguage("en-US"));

    expect(screen.getByDisplayValue("保留这个草稿")).toBeInTheDocument();
    expect(screen.getByLabelText("Workflow canvas")).toBeInTheDocument();
  });

  it("localizes workflow chrome and mock content in English", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(<WorkflowSettings />);

    expect(await screen.findByText("Code review workflow")).toBeInTheDocument();
    expect(screen.getByLabelText("Workflow canvas")).toBeInTheDocument();
    expect(
      screen.getByText("Scroll to zoom · Drag to pan · Nodes snap to grid"),
    ).toBeInTheDocument();

    await user.click(screen.getAllByRole("button", { name: "Test run" })[0]);

    expect(await screen.findByText("Simulation successful")).toBeInTheDocument();
    expect(screen.getByText(/Found 2 suggestions and no blocking issues./)).toBeInTheDocument();
  });
});
