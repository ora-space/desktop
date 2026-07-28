import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import { appI18n } from "../../i18n/i18n-instance";
import { WorkflowSettings } from "./workflow-settings";

describe("WorkflowSettings", () => {
  afterEach(async () => {
    Reflect.deleteProperty(document, "elementFromPoint");
    await appI18n.changeLanguage("zh-CN");
  });

  it("loads the mock graph and exposes a deterministic preview", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);

    expect(screen.getByText("正在加载 mock 工作流…")).toBeInTheDocument();
    expect(await screen.findByText("OpenSpec 模式")).toBeInTheDocument();
    expect(screen.getByLabelText("工作流画布")).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "输入" })).toBeInTheDocument();
    expect(screen.getByRole("group", { name: "处理" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "触发器" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "数据来源" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "LLM" })).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "代码" })).toBeInTheDocument();

    await user.click(screen.getAllByRole("button", { name: "测试运行" })[0]);

    expect(await screen.findByText("模拟运行成功")).toBeInTheDocument();
    expect(screen.getByText(/主规格已同步，变更已归档。/)).toBeInTheDocument();
  });

  it("zooms around the pointer with the mouse wheel", async () => {
    render(<WorkflowSettings />);
    const canvas = await screen.findByLabelText("工作流画布");

    expect(screen.getByText("100%")).toBeInTheDocument();
    fireEvent.wheel(canvas, { deltaY: -200, clientX: 240, clientY: 180 });

    await waitFor(() => {
      expect(screen.queryByText("100%")).not.toBeInTheDocument();
    });
  });

  it("pans the board by dragging empty canvas space", async () => {
    render(<WorkflowSettings />);
    const canvas = await screen.findByLabelText("工作流画布");
    canvas.setPointerCapture = () => {};
    const stage = canvas.querySelector<HTMLElement>(".origin-top-left");

    expect(stage?.style.transform).toBe("translate(32px, 32px) scale(1)");
    fireEvent.pointerDown(canvas, { button: 0, clientX: 100, clientY: 100, pointerId: 1 });
    fireEvent.pointerMove(canvas, { clientX: 150, clientY: 170, pointerId: 1 });
    fireEvent.pointerUp(canvas, { clientX: 150, clientY: 170, pointerId: 1 });

    await waitFor(() => {
      expect(stage?.style.transform).toBe("translate(82px, 102px) scale(1)");
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

    await user.click(screen.getByRole("button", { name: "模板转换" }));

    expect(screen.getAllByText("模板转换 1")).toHaveLength(2);
    const addedNode = screen.getByLabelText("模板转换节点: 模板转换 1");
    expect({
      left: addedNode.style.left,
      top: addedNode.style.top,
    }).toEqual({
      left: "253px",
      top: "207px",
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
    const dataTransfer = createDataTransfer();
    const toolButton = screen.getByRole("button", { name: "工具" });

    fireEvent.dragStart(toolButton, { dataTransfer });
    fireEvent.dragEnter(canvas, { dataTransfer });

    expect(screen.getByText("释放以添加节点")).toBeInTheDocument();

    fireEvent.dragOver(canvas, { clientX: 500, clientY: 350, dataTransfer });
    const dropEvent = new MouseEvent("drop", {
      bubbles: true,
      cancelable: true,
      clientX: 500,
      clientY: 350,
    });
    Object.defineProperty(dropEvent, "dataTransfer", { value: dataTransfer });
    fireEvent(canvas, dropEvent);

    const addedNode = screen.getByLabelText("工具节点: 工具 1");
    expect({
      left: addedNode.style.left,
      top: addedNode.style.top,
    }).toEqual({
      left: "353px",
      top: "257px",
    });
    expect(screen.queryByText("释放以添加节点")).not.toBeInTheDocument();
  });

  it("deletes workflow connections by double-click or keyboard", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);

    const codeReviewWorkflow = await screen.findByText("代码审查工作流");
    await user.click(codeReviewWorkflow.closest("button")!);

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

  it("moves a selected connection endpoint to another node", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);

    const codeReviewWorkflow = await screen.findByText("代码审查工作流");
    await user.click(codeReviewWorkflow.closest("button")!);

    const connection = await screen.findByRole("button", {
      name: "选择从开始到理解改动的连线",
    });
    await user.click(connection);

    const sourceHandle = screen.getByRole("button", {
      name: "移动连线起点：开始",
    });
    const nextSourceHandle = screen.getByRole("button", {
      name: "从运行检查开始连接",
    });
    const nextSourceNode = screen.getByLabelText("代码节点: 运行检查");
    sourceHandle.setPointerCapture = () => {};
    const elementFromPoint = vi.fn(() => nextSourceNode);
    Object.defineProperty(document, "elementFromPoint", {
      configurable: true,
      value: elementFromPoint,
    });

    fireEvent.pointerDown(sourceHandle, {
      pointerId: 1,
      clientX: 302,
      clientY: 347,
    });
    fireEvent.pointerMove(screen.getByLabelText("工作流连线"), {
      pointerId: 1,
      clientX: 1168,
      clientY: 153,
    });

    expect(nextSourceNode.className).toContain("border-ring");
    expect(nextSourceHandle.className).toContain("ring-2");

    fireEvent.pointerUp(screen.getByLabelText("工作流连线"), {
      pointerId: 1,
      clientX: 1168,
      clientY: 153,
    });

    expect(screen.getByRole("button", {
      name: "选择从运行检查到理解改动的连线",
    })).toBeInTheDocument();
    expect(screen.queryByRole("button", {
      name: "选择从开始到理解改动的连线",
    })).not.toBeInTheDocument();

    const targetHandle = screen.getByRole("button", {
      name: "移动连线终点：理解改动",
    });
    const nextTargetHandle = screen.getByRole("button", {
      name: "连接到质量门禁",
    });
    const nextTargetNode = screen.getByLabelText("条件分支节点: 质量门禁");
    targetHandle.setPointerCapture = () => {};
    elementFromPoint.mockReturnValue(nextTargetNode);

    fireEvent.pointerDown(targetHandle, {
      pointerId: 2,
      clientX: 356,
      clientY: 249,
    });
    fireEvent.pointerMove(screen.getByLabelText("工作流连线"), {
      pointerId: 2,
      clientX: 650,
      clientY: 249,
    });

    expect(nextTargetNode.className).toContain("border-ring");
    expect(nextTargetHandle.className).toContain("ring-2");

    fireEvent.pointerUp(screen.getByLabelText("工作流连线"), {
      pointerId: 2,
      clientX: 650,
      clientY: 249,
    });

    expect(screen.getByRole("button", {
      name: "选择从运行检查到质量门禁的连线",
    })).toBeInTheDocument();
  });

  it("creates a workflow from the left manager and allows renaming it", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);

    await screen.findByText("OpenSpec 模式");
    await user.click(screen.getByRole("button", { name: "新建工作流" }));
    const nameInput = await screen.findByDisplayValue("新工作流 7");
    await user.clear(nameInput);
    await user.type(nameInput, "发布复盘");

    expect(screen.getByDisplayValue("发布复盘")).toBeInTheDocument();
    expect(screen.getByText("7 个工作流")).toBeInTheDocument();
  });

  it("shows the OpenSpec preset with the same lifecycle as OpenSpec mode", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);

    const openspecWorkflow = await screen.findByText("OpenSpec 模式");
    await user.click(openspecWorkflow.closest("button")!);

    expect(screen.getByDisplayValue("OpenSpec 模式")).toBeInTheDocument();
    expect(screen.getByLabelText("LLM节点: 探索需求")).toBeInTheDocument();
    expect(screen.getByLabelText("LLM节点: 创建提案")).toBeInTheDocument();
    expect(screen.getByLabelText("LLM节点: 实施变更")).toBeInTheDocument();
    expect(screen.getByLabelText("LLM节点: 同步主规格")).toBeInTheDocument();
    expect(screen.getByLabelText("工具节点: 归档变更")).toBeInTheDocument();

    fireEvent.focus(screen.getByLabelText("LLM节点: 探索需求"));

    expect(screen.getByLabelText("命令 / Skill")).toHaveValue("$openspec-explore");
    expect(screen.getByLabelText("执行指令")).toHaveValue(
      "使用 openspec-explore skill 检查相关代码与现有规格，澄清用户路径和边界条件。此阶段只探索，不写实现。",
    );
    expect(screen.getByText("CI 失败修复")).toBeInTheDocument();
    expect(screen.getByText("依赖安全升级")).toBeInTheDocument();
  });

  it("shows type-specific configuration for triggers, data sources, and code", async () => {
    const user = userEvent.setup();
    render(<WorkflowSettings />);

    fireEvent.focus(await screen.findByLabelText("触发器节点: 开始变更"));
    expect(screen.getByLabelText("触发方式")).toHaveTextContent("Manual");

    const ciWorkflow = screen.getByText("CI 失败修复");
    await user.click(ciWorkflow.closest("button")!);
    fireEvent.focus(screen.getByLabelText("数据来源节点: 拉取失败日志"));
    expect(screen.getByLabelText("数据来源")).toHaveTextContent("GitHub");

    const dependencyWorkflow = screen.getByText("依赖安全升级");
    await user.click(dependencyWorkflow.closest("button")!);
    fireEvent.focus(screen.getByLabelText("代码节点: 扫描依赖风险"));
    expect(screen.getByLabelText("运行语言")).toHaveTextContent("Shell");
    expect(screen.getByLabelText("命令 / Skill")).toHaveValue(
      "cargo audit\npnpm audit --prod",
    );
  });

  it("localizes workflow chrome and mock content in English", async () => {
    await appI18n.changeLanguage("en-US");
    const user = userEvent.setup();
    render(<WorkflowSettings />);

    expect(await screen.findByText("OpenSpec mode")).toBeInTheDocument();
    expect(screen.getByLabelText("Workflow canvas")).toBeInTheDocument();
    expect(screen.getByText("Scroll to zoom · Drag to pan")).toBeInTheDocument();

    await user.click(screen.getAllByRole("button", { name: "Test run" })[0]);

    expect(await screen.findByText("Simulation successful")).toBeInTheDocument();
    expect(screen.getByText(/main specs were synced, and the change was archived./)).toBeInTheDocument();
  });
});

/** Provides the subset of native drag data behavior used by the workflow catalog. */
function createDataTransfer(): DataTransfer {
  const values = new Map<string, string>();
  const types: string[] = [];
  return {
    dropEffect: "none",
    effectAllowed: "all",
    files: [] as unknown as FileList,
    items: [] as unknown as DataTransferItemList,
    types,
    clearData: (format?: string) => {
      if (format === undefined) {
        values.clear();
        types.splice(0);
        return;
      }
      values.delete(format);
      const index = types.indexOf(format);
      if (index >= 0) {
        types.splice(index, 1);
      }
    },
    getData: (format: string) => values.get(format) ?? "",
    setData: (format: string, data: string) => {
      values.set(format, data);
      if (!types.includes(format)) {
        types.push(format);
      }
    },
    setDragImage: () => {},
  };
}
