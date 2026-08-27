import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import type { Node } from "@xyflow/react";
import {
  createMockWorkflowCapabilities,
  type WorkflowNodeData,
} from "@ora/workflow-mock";
import { appI18n } from "../../i18n/i18n-instance";
import { AppI18nProvider } from "../../i18n/i18n";
import { WorkflowInspector } from "./workflow-inspector";

const LONG_MODEL_LABEL =
  "OpenCode · deepseek/deepseek-v4-pro-with-an-extremely-long-model-identifier";

/** Builds one Agent node whose long labels would overflow a narrow inspector without min-width constraints. */
function createAgentNode(): Node<WorkflowNodeData, "workflow"> {
  return {
    id: "agent-1",
    type: "workflow",
    position: { x: 0, y: 0 },
    data: {
      kind: "agent",
      title: "探索",
      description: "只读探索项目现状和影响范围",
      agentConfig: {
        schemaVersion: 3,
        executor: {
          agentCli: "ora-space.opencode",
          modelId:
            "deepseek/deepseek-v4-pro-with-an-extremely-long-model-identifier",
        },
        roleId: "Researcher",
        skills: [{ skillId: "openspec-explore", enabled: true }],
        mcps: [],
        prompt:
          "阅读相关代码、文档和现有规范，输出现状、约束、风险与可选路径。",
      },
    },
  };
}

/** Builds one Condition node with a structured branch rule to exercise the IF/ELSE panel. */
function createConditionNode(): Node<WorkflowNodeData, "workflow"> {
  return {
    id: "condition-1",
    type: "workflow",
    position: { x: 0, y: 0 },
    data: {
      kind: "condition",
      title: "质量门禁",
      description: "判断是否需要执行测试",
      conditionBranches: [
        {
          conditions: [
            { variable: "工具1.exit_code", operator: "equals", value: "0" },
          ],
        },
      ],
      instruction: "根据改动类型选择后续路径。",
    },
  };
}

/** Builds one Tool node to exercise the tool-card panel. */
function createToolNode(): Node<WorkflowNodeData, "workflow"> {
  return {
    id: "tool-1",
    type: "workflow",
    position: { x: 0, y: 0 },
    data: {
      kind: "tool",
      title: "运行检查",
      description: "执行格式化、类型检查和测试",
      tool: "Terminal",
      operation: "run_command",
      instruction: "运行与改动范围匹配的最小验证集。",
    },
  };
}

/** Builds one Junction node to exercise the merge-strategy panel. */
function createJunctionNode(): Node<WorkflowNodeData, "workflow"> {
  return {
    id: "junction-1",
    type: "workflow",
    position: { x: 0, y: 0 },
    data: {
      kind: "junction",
      title: "审核汇合",
      description: "等待全部审核分支完成",
      waitStrategy: "all",
      failureStrategy: "fail",
      instruction: "合并审核结果。",
    },
  };
}

/** Builds one Loop node to exercise the retry panel. */
function createLoopNode(): Node<WorkflowNodeData, "workflow"> {
  return {
    id: "loop-1",
    type: "workflow",
    position: { x: 0, y: 0 },
    data: {
      kind: "loop",
      title: "修复循环",
      description: "修复后重新验证",
      maxAttempts: 3,
      exitCondition: "verification.status == passed",
      instruction: "修复失败后回到验证。",
    },
  };
}

/** Builds one Start node to exercise the inputs panel. */
function createStartNode(): Node<WorkflowNodeData, "workflow"> {
  return {
    id: "start-1",
    type: "workflow",
    position: { x: 0, y: 0 },
    data: {
      kind: "start",
      title: "开始",
      description: "定义工作流输入",
      instruction: "检查当前工作区的未提交改动",
    },
  };
}

/** Mounts the inspector inside a fixed-width clip container that mirrors the editor rail. */
function renderNarrowInspector(): HTMLElement {
  const capabilities = createMockWorkflowCapabilities("zh-CN", [
    {
      agentCli: "ora-space.opencode",
      modelId:
        "deepseek/deepseek-v4-pro-with-an-extremely-long-model-identifier",
      label: LONG_MODEL_LABEL,
    },
  ]);
  const container = document.createElement("div");
  container.dataset.testid = "narrow-inspector-host";
  container.style.width = "240px";
  container.style.overflow = "hidden";
  container.className = "flex min-h-0 min-w-0 flex-col";
  document.body.append(container);

  render(
    <AppI18nProvider>
      <WorkflowInspector
        node={createAgentNode()}
        capabilities={capabilities}
        onUpdate={() => undefined}
        onDelete={() => undefined}
        onCloseNode={() => undefined}
      />
    </AppI18nProvider>,
    { container },
  );
  return container;
}

describe("WorkflowInspector layout", () => {
  it("keeps picker chevrons and skill controls inside a narrow clipped rail", async () => {
    await appI18n.changeLanguage("zh-CN");
    const host = renderNarrowInspector();
    const inspector = host.querySelector("[data-workflow-inspector]");
    expect(inspector).not.toBeNull();
    expect(inspector).toHaveClass("min-w-0", "w-full", "overflow-hidden");

    const modelTrigger = screen.getByLabelText("Agent 模型");
    expect(modelTrigger).toHaveClass(
      "min-w-0",
      "shrink",
      "overflow-hidden",
      "w-full",
    );
    expect(
      within(modelTrigger).getByTestId("workflow-agent-model-chevron"),
    ).toBeInTheDocument();

    const roleTrigger = screen.getByLabelText("角色");
    expect(roleTrigger).toHaveClass(
      "min-w-0",
      "shrink",
      "overflow-hidden",
      "w-full",
    );
    expect(
      within(roleTrigger).getByTestId("workflow-agent-role-chevron"),
    ).toBeInTheDocument();

    const addSkill = screen.getByRole("button", { name: "添加 Skill" });
    expect(addSkill).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "添加 MCP" }),
    ).toBeInTheDocument();
    expect(screen.getByText(/1\/1/)).toBeInTheDocument();
    expect(screen.getByText("暂未配置 MCP（可选）")).toBeInTheDocument();
    expect(
      screen.getByRole("switch", {
        name: "启用或禁用 openspec-explore",
      }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", {
        name: "移除 openspec-explore",
      }),
    ).toBeInTheDocument();

    host.remove();
  });
});

/** Applies inspector updates to local state so interactions like adding rows take effect. */
function StatefulInspectorHarness({
  node,
  capabilities,
}: {
  node: Node<WorkflowNodeData, "workflow">;
  capabilities: ReturnType<typeof createMockWorkflowCapabilities>;
}) {
  const [current, setCurrent] = useState(node);
  return (
    <AppI18nProvider>
      <WorkflowInspector
        node={current}
        capabilities={capabilities}
        onUpdate={setCurrent}
        onDelete={() => undefined}
        onCloseNode={() => undefined}
      />
    </AppI18nProvider>
  );
}

describe("WorkflowInspector kind-specific layouts", () => {
  it("renders a merge panel with wait and failure strategies for junction nodes", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    render(
      <StatefulInspectorHarness
        node={createJunctionNode()}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "汇合节点" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("等待策略")).toHaveTextContent("全部分支完成");
    expect(screen.getByLabelText("失败策略")).toHaveTextContent(
      "任一失败则失败",
    );

    await user.click(screen.getByRole("button", { name: "高级设置" }));
    expect(screen.getByLabelText("执行指令")).toHaveValue("合并审核结果。");
  });

  it("renders a loop panel with max attempts and exit condition", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    render(
      <StatefulInspectorHarness
        node={createLoopNode()}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "循环节点" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("最大次数")).toHaveValue(3);
    expect(screen.getByLabelText("退出条件")).toHaveValue(
      "verification.status == passed",
    );

    await user.click(screen.getByRole("button", { name: "高级设置" }));
    expect(screen.getByLabelText("执行指令")).toHaveValue(
      "修复失败后回到验证。",
    );
  });

  it("renders an IF/ELSE branch panel for condition nodes", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    render(
      <StatefulInspectorHarness
        node={createConditionNode()}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "条件分支节点" }),
    ).toBeInTheDocument();
    expect(screen.getByText("分支 1")).toBeInTheDocument();
    expect(screen.getByLabelText("分支 1 逻辑")).toHaveTextContent(
      "满足以下所有条件",
    );
    expect(screen.getByLabelText("变量 1")).toHaveValue("工具1.exit_code");
    expect(screen.getByLabelText("条件 1")).toHaveTextContent("等于");
    expect(screen.getByLabelText("值 1")).toHaveValue("0");
    const notToggle = screen.getByRole("button", { name: "切换条件 1 的非" });
    expect(notToggle).toHaveAttribute("aria-pressed", "false");
    await user.click(notToggle);
    expect(notToggle).toHaveAttribute("aria-pressed", "true");
    expect(
      screen.getByRole("button", { name: "添加条件" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "添加分支" }),
    ).toBeInTheDocument();
    expect(screen.getByText("其他情况")).toBeInTheDocument();
    expect(screen.getByText("默认分支")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "高级设置" }));
    expect(screen.getByLabelText("执行指令")).toHaveValue(
      "根据改动类型选择后续路径。",
    );
  });

  it("renders a tool panel with operation and parameters for tool nodes", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    render(
      <StatefulInspectorHarness
        node={createToolNode()}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "工具节点" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("工具")).toHaveTextContent("Terminal");
    expect(screen.getByLabelText("操作")).toHaveTextContent("执行命令");
    expect(screen.getByRole("heading", { name: "参数" })).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "添加参数" }));
    expect(screen.getByLabelText("参数名 1")).toBeInTheDocument();
    expect(screen.getByLabelText("参数值 1")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "高级设置" }));
    expect(screen.getByLabelText("执行指令")).toHaveValue(
      "运行与改动范围匹配的最小验证集。",
    );
  });

  it("renders a start panel with trigger, input variables, and available variables", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    render(
      <StatefulInspectorHarness
        node={createStartNode()}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    expect(
      screen.getByRole("heading", { name: "开始节点" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("名称")).toHaveValue("开始");
    expect(screen.getByLabelText("触发方式")).toHaveTextContent(
      "Merge Request",
    );
    expect(
      screen.getByRole("heading", { name: "输入变量" }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "添加变量" }));
    expect(screen.getByLabelText("变量名 1")).toBeInTheDocument();
    expect(screen.getByLabelText("默认值 1")).toBeInTheDocument();

    expect(
      screen.getByRole("heading", { name: "可用变量" }),
    ).toBeInTheDocument();
    expect(screen.getByText("repository")).toBeInTheDocument();
    expect(screen.getByText("changed_files")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "高级设置" }));
    expect(screen.getByLabelText("执行指令")).toHaveValue(
      "检查当前工作区的未提交改动",
    );
  });
});
