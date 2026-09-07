import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { describe, expect, it } from "vitest";
import type { Node } from "@xyflow/react";
import {
  createMockWorkflowCapabilities,
  resolveConditionCases,
  type WorkflowNodeData,
  type WorkflowVariableCatalogEntry,
} from "@ora/workflow-mock";
import { appI18n } from "../../i18n/i18n-instance";
import { AppI18nProvider } from "../../i18n/i18n";
import { WorkflowInspector } from "./workflow-inspector";
import { AGENT_REF } from "../../test/agent-identity";

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
          agentCli: AGENT_REF.opencode,
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
      cases: [
        {
          id: "case-1",
          conditions: [
            {
              variableSelector: ["工具1", "exit_code"],
              operator: "equals",
              value: "0",
            },
          ],
        },
      ],
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
      input: "检查当前工作区的未提交改动",
    },
  };
}

/** Mounts the inspector inside a fixed-width clip container that mirrors the editor rail. */
function renderNarrowInspector(): HTMLElement {
  const capabilities = createMockWorkflowCapabilities("zh-CN", [
    {
      agentCli: AGENT_REF.opencode,
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
        variableCatalog={[]}
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

    const orderedSections = [
      screen.getByText("Agent 模型"),
      screen.getByText("自定义 Prompt"),
      screen.getByText("角色"),
      screen.getByText("Skills"),
      screen.getByText("MCP"),
      screen.getByText("交互模式"),
      screen.getByText("结构化输出"),
    ];
    for (let index = 1; index < orderedSections.length; index += 1) {
      expect(
        orderedSections[index - 1].compareDocumentPosition(
          orderedSections[index],
        ) & globalThis.Node.DOCUMENT_POSITION_FOLLOWING,
      ).not.toBe(0);
    }

    host.remove();
  });
});

/** Applies inspector updates to local state so interactions like adding rows take effect. */
function StatefulInspectorHarness({
  node,
  capabilities,
  variableCatalog = [
    {
      selector: ["工具1", "exit_code"],
      sourceNodeId: "工具1",
      variableName: "exit_code",
      valueType: "integer",
    },
    {
      selector: ["writer", "output"],
      sourceNodeId: "writer",
      variableName: "output",
      valueType: "string",
    },
    {
      selector: ["writer", "text"],
      sourceNodeId: "writer",
      variableName: "text",
      valueType: "string",
    },
  ],
}: {
  node: Node<WorkflowNodeData, "workflow">;
  capabilities: ReturnType<typeof createMockWorkflowCapabilities>;
  variableCatalog?: WorkflowVariableCatalogEntry[];
}) {
  const [current, setCurrent] = useState(node);
  return (
    <AppI18nProvider>
      <WorkflowInspector
        variableCatalog={variableCatalog}
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
  it("starts IF empty and adds a fully blank condition row", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    const conditionNode = createConditionNode();
    render(
      <StatefulInspectorHarness
        node={{
          ...conditionNode,
          data: {
            ...conditionNode.data,
            cases: [{ id: "case-1", logic: "and", conditions: [] }],
          },
        }}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    expect(screen.queryByLabelText("变量 1")).not.toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "添加条件" }));
    expect(screen.getByLabelText("变量 1")).toHaveTextContent(
      "如 工具1.exit_code",
    );
    expect(screen.getByLabelText("条件 1")).toHaveTextContent("选择条件");
    expect(screen.getByLabelText("值 1")).toHaveValue("");
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
      screen.getByRole("heading", { name: "质量门禁" }),
    ).toBeInTheDocument();
    expect(screen.getByText("IF")).toBeInTheDocument();
    const firstVariable = screen.getByLabelText("变量 1");
    expect(
      firstVariable.querySelector('[data-workflow-variable-part="node-name"]'),
    ).toHaveTextContent("工具1");
    expect(
      firstVariable.querySelector(
        '[data-workflow-variable-part="variable-name"]',
      ),
    ).toHaveTextContent("exit_code");
    expect(screen.getByLabelText("条件 1")).toHaveTextContent("等于");
    expect(screen.getByLabelText("值 1")).toHaveValue("0");
    expect(
      screen.getByRole("button", { name: "添加条件" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "ELIF" })).toBeInTheDocument();
    expect(screen.getByText("ELSE")).toBeInTheDocument();

    expect(screen.queryByLabelText("执行指令")).not.toBeInTheDocument();
    expect(
      screen.queryByLabelText("模拟执行耗时 (ms)"),
    ).not.toBeInTheDocument();

    await user.click(firstVariable);
    const option = await screen.findByRole("option", { name: "writer.output" });
    expect(option.className).toContain(
      "[&_[data-workflow-variable-part=variable-mark]]:text-blue-600!",
    );
    expect(
      option.querySelector('[data-workflow-variable-part="variable-mark"]'),
    ).toHaveClass("text-blue-600");
    expect(
      option.querySelector('[data-workflow-variable-part="variable-name"]'),
    ).toHaveClass("text-foreground");
    await user.keyboard("{Escape}");
  });

  it("places the shared AND/OR selector between conditions in one branch", async () => {
    await appI18n.changeLanguage("zh-CN");
    const conditionNode = createConditionNode();
    render(
      <StatefulInspectorHarness
        node={{
          ...conditionNode,
          data: {
            ...conditionNode.data,
            cases: [
              {
                id: "case-1",
                logic: "or",
                conditions: [
                  ...resolveConditionCases(conditionNode.data)[0].conditions,
                  {
                    variableSelector: ["writer", "text"],
                    operator: "contains",
                    value: "done",
                  },
                ],
              },
            ],
          },
        }}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    expect(screen.getByLabelText("分支 1 逻辑 · 1")).toHaveTextContent("OR");
    const secondVariable = screen.getByLabelText("变量 2");
    expect(
      secondVariable.querySelector(
        '[data-workflow-variable-part="variable-name"]',
      ),
    ).toHaveTextContent("text");
  });

  it("renders and edits an agent structured output contract", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    const structuredAgent: Node<WorkflowNodeData, "workflow"> = {
      ...createAgentNode(),
      data: {
        ...createAgentNode().data,
        agentConfig: {
          ...createAgentNode().data.agentConfig!,
          outputContract: {
            type: "structured",
            schema: {
              type: "object",
              properties: { approved: { type: "boolean" } },
            },
          },
        },
      },
    };
    render(
      <StatefulInspectorHarness
        node={structuredAgent}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    expect(screen.getByRole("switch", { name: "结构化输出" })).toBeChecked();
    expect(screen.queryByLabelText("文本暴露")).not.toBeInTheDocument();
    expect(screen.getByText("structured_output")).toBeInTheDocument();
    expect(screen.getByText("approved")).toBeInTheDocument();
    expect(screen.getByText("boolean")).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "配置" }));
    expect(screen.getByRole("dialog")).toHaveTextContent("结构化输出 Schema");
    await user.click(screen.getByRole("tab", { name: "JSON Schema" }));
    const schemaEditor = screen.getByLabelText("JSON Schema");
    fireEvent.change(schemaEditor, {
      target: {
        value: '{"type":"object","properties":{"created":{"type":"date"}}}',
      },
    });
    await user.click(screen.getByRole("button", { name: "保存" }));
    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("有效 JSON Schema");
    fireEvent.change(schemaEditor, {
      target: {
        value:
          '{"type":"object","properties":{"score":{"type":"number","description":"评分"}},"required":["score"],"additionalProperties":false}',
      },
    });
    await user.click(screen.getByRole("button", { name: "保存" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByText("score")).toBeInTheDocument();
    expect(screen.getByText("评分")).toBeInTheDocument();
    expect(screen.getByText("必填")).toBeInTheDocument();
  });

  it("visually configures nested structured output fields", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    render(
      <StatefulInspectorHarness
        node={createAgentNode()}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    await user.click(screen.getByRole("switch", { name: "结构化输出" }));
    expect(screen.getByText("结构化输出尚未配置")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "配置" }));
    await user.click(screen.getByRole("button", { name: "添加字段" }));
    expect(
      screen.getByRole("button", { name: "为 field_1 添加子字段" }),
    ).toBeDisabled();
    await user.click(screen.getByRole("button", { name: "编辑字段 field_1" }));
    const fieldName = screen.getByLabelText("字段名");
    await user.clear(fieldName);
    await user.type(fieldName, "feedback");
    await user.click(screen.getByRole("combobox", { name: "字段类型" }));
    await user.click(
      await screen.findByRole("option", { name: "array[object]" }),
    );
    await user.click(screen.getByRole("switch", { name: "必填" }));
    await user.type(screen.getByLabelText("字段描述"), "餐厅回复");
    await user.click(screen.getByRole("button", { name: "编辑字段 feedback" }));
    const addChild = screen.getByRole("button", {
      name: "为 feedback 添加子字段",
    });
    expect(addChild).toBeEnabled();
    await user.click(addChild);
    await user.click(screen.getByRole("button", { name: "编辑字段 field_1" }));
    await user.clear(screen.getByLabelText("字段名"));
    await user.type(screen.getByLabelText("字段名"), "reply");
    await user.click(screen.getByRole("button", { name: "保存" }));

    expect(screen.getByText("feedback")).toBeInTheDocument();
    expect(screen.getByText("array[object]")).toBeInTheDocument();
    expect(screen.getByText("reply")).toBeInTheDocument();
    expect(screen.getByText("餐厅回复")).toBeInTheDocument();
  });

  it("always exposes output and enables structured output independently", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    render(
      <StatefulInspectorHarness
        node={createAgentNode()}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    expect(
      screen.getByRole("switch", { name: "结构化输出" }),
    ).not.toBeChecked();
    expect(screen.queryByText("使用输出策略")).not.toBeInTheDocument();
    expect(screen.queryByLabelText("输出策略")).not.toBeInTheDocument();
    expect(screen.getByLabelText(/\d+ 个字符/)).toBeInTheDocument();

    await user.click(screen.getByRole("switch", { name: "结构化输出" }));
    expect(screen.getByText("结构化输出尚未配置")).toBeInTheDocument();

    await user.click(screen.getByLabelText("插入变量"));
    expect(
      screen.getByLabelText("自定义 Prompt").dataset.composerText,
    ).toContain("/");
    await user.click(screen.getByRole("option", { name: "writer.output" }));
    const prompt = screen.getByLabelText("自定义 Prompt");
    expect(prompt).toHaveTextContent("writer");
    expect(prompt.dataset.composerText).toContain("{{#writer.output#}}");
    expect(prompt.querySelector("[data-workflow-prompt-variable]")).toHaveClass(
      "composer-mention",
    );
    expect(
      prompt.querySelector('[data-workflow-variable-part="node-name"]'),
    ).toHaveTextContent("writer");
    expect(
      prompt.querySelector('[data-workflow-variable-part="variable-mark"]'),
    ).toHaveTextContent("{x}");
    expect(
      prompt.querySelector('[data-workflow-variable-part="variable-name"]'),
    ).toHaveTextContent("output");
    expect(
      prompt.querySelector('[data-workflow-variable-part="variable-type"]'),
    ).not.toBeInTheDocument();

    await user.clear(prompt);
    await user.type(prompt, "输出完整方案。/");
    expect(
      screen.getByRole("listbox", { name: "插入变量" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("option", { name: "writer.output" }));
    expect(prompt).toHaveTextContent("输出完整方案。");
    expect(prompt).toHaveTextContent("output");
    expect(prompt.dataset.composerText).toBe(
      "输出完整方案。{{#writer.output#}} ",
    );

    const writeText = vi.spyOn(navigator.clipboard, "writeText");
    await user.click(screen.getByLabelText("复制文本"));
    expect(writeText).toHaveBeenCalledWith(
      expect.stringContaining("{{#writer.output#}}"),
    );
    writeText.mockRestore();

    await user.click(screen.getByLabelText("放大文本框"));
    expect(screen.getByRole("dialog")).toHaveTextContent("自定义 Prompt");
  });

  it("restores persisted prompt variables with their rich node and type display", async () => {
    await appI18n.changeLanguage("zh-CN");
    const node = createAgentNode();
    render(
      <StatefulInspectorHarness
        node={{
          ...node,
          data: {
            ...node.data,
            agentConfig: {
              ...node.data.agentConfig!,
              prompt: "使用 {{#writer.output#}} 继续处理。",
            },
          },
        }}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    const prompt = screen.getByLabelText("自定义 Prompt");
    expect(prompt.dataset.composerText).toBe(
      "使用 {{#writer.output#}} 继续处理。",
    );
    await waitFor(() => {
      expect(
        prompt.querySelector('[data-workflow-variable-part="node-name"]'),
      ).toHaveTextContent("writer");
      expect(
        prompt.querySelector('[data-workflow-variable-part="variable-name"]'),
      ).toHaveTextContent("output");
      expect(
        prompt.querySelector('[data-workflow-variable-part="variable-type"]'),
      ).not.toBeInTheDocument();
    });
  });

  it("groups prompt variables by global scope and producer node title", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    render(
      <StatefulInspectorHarness
        node={createAgentNode()}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
        variableCatalog={[
          {
            selector: ["sys", "workflow_id"],
            sourceNodeId: "sys",
            scope: "global",
            variableName: "workflow_id",
            valueType: "string",
          },
          {
            selector: ["research", "output"],
            sourceNodeId: "research",
            sourceNodeTitle: "资料检索",
            scope: "node",
            variableName: "output",
            valueType: "string",
          },
          {
            selector: ["research", "structured_output", "summary"],
            sourceNodeId: "research",
            sourceNodeTitle: "资料检索",
            scope: "node",
            variableName: "structured_output.summary",
            valueType: "string",
          },
        ]}
      />,
    );

    await user.click(screen.getByLabelText("插入变量"));
    const menu = screen.getByRole("listbox", { name: "插入变量" });
    expect(within(menu).getByLabelText("全局变量")).toHaveTextContent(
      "workflow_id",
    );
    expect(within(menu).getByLabelText("资料检索")).toHaveTextContent(
      /output.*structured_output\.summary/,
    );

    await user.click(
      within(menu).getByRole("option", { name: "research.output" }),
    );
    expect(
      screen.getByLabelText("自定义 Prompt").dataset.composerText,
    ).toContain("{{#research.output#}}");
  });

  it("edits output result bindings", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    const outputNode: Node<WorkflowNodeData, "workflow"> = {
      id: "output-1",
      type: "workflow",
      position: { x: 0, y: 0 },
      data: {
        kind: "output",
        title: "输出",
        description: "",
        outputs: [{ name: "summary", variableSelector: ["writer", "text"] }],
      },
    };
    render(
      <StatefulInspectorHarness
        node={outputNode}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    expect(screen.getByLabelText("结果名 1")).toHaveValue("summary");
    const outputVariable = screen.getByLabelText("结果变量 1");
    expect(
      outputVariable.querySelector(
        '[data-workflow-variable-part="variable-name"]',
      ),
    ).toHaveTextContent("text");
    expect(screen.queryByLabelText("执行指令")).not.toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: "添加结果" }));
    expect(screen.getByLabelText("结果名 2")).toHaveValue("result-2");
  });

  it("renders a start panel with input variables and an initial prompt", async () => {
    await appI18n.changeLanguage("zh-CN");
    const user = userEvent.setup();
    render(
      <StatefulInspectorHarness
        node={createStartNode()}
        capabilities={createMockWorkflowCapabilities("zh-CN")}
      />,
    );

    expect(screen.getByRole("heading", { name: "开始" })).toBeInTheDocument();
    expect(screen.queryByLabelText("名称")).not.toBeInTheDocument();
    expect(screen.getByLabelText("说明")).toHaveValue("定义工作流输入");
    await user.dblClick(screen.getByRole("button", { name: "开始" }));
    const title = screen.getByLabelText("名称");
    await user.clear(title);
    await user.type(title, "启动{Enter}");
    expect(screen.getByRole("heading", { name: "启动" })).toBeInTheDocument();
    expect(
      screen.getByRole("heading", { name: "输入变量" }),
    ).toBeInTheDocument();
    expect(screen.getByText("尚未添加输入字段")).toBeInTheDocument();
    expect(screen.getByLabelText("初始 Prompt")).toHaveValue(
      "检查当前工作区的未提交改动",
    );

    await user.click(screen.getByRole("button", { name: "添加变量" }));
    expect(
      screen.getByRole("heading", { name: "新增变量" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("combobox", { name: "字段类型" }));
    for (const fieldType of [
      "文本",
      "段落",
      "下拉选项",
      "数字",
      "复选框",
      "单文件",
      "文件列表",
      "JSON",
    ]) {
      expect(
        await screen.findByRole("option", { name: new RegExp(fieldType) }),
      ).toBeInTheDocument();
    }
    await user.click(screen.getByRole("option", { name: /文本/ }));
    await user.type(screen.getByLabelText("变量名称"), "priority");
    await user.type(screen.getByLabelText(/显示名称/), "优先级");
    const maxLength = screen.getByLabelText(/最大长度/);
    await user.type(maxLength, "1");
    const defaultValue = screen.getByLabelText(/初始值/);
    await user.type(defaultValue, "too long");
    await user.click(screen.getByRole("button", { name: "保存" }));
    expect(defaultValue).toHaveAttribute("aria-invalid", "true");
    expect(screen.getByText("内容不能超过 1 个字符。")).toBeInTheDocument();
    await user.clear(maxLength);
    await user.type(maxLength, "20");
    await user.click(screen.getByRole("button", { name: "保存" }));
    expect(screen.getByText("priority")).toBeInTheDocument();
    expect(screen.getByText(/优先级/)).toBeInTheDocument();
    expect(screen.getByText(/文本 · 最多 20 字符/)).toHaveTextContent(
      "too long",
    );

    await user.click(screen.getByRole("button", { name: "编辑变量 priority" }));
    expect(
      screen.getByRole("heading", { name: "编辑变量" }),
    ).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "取消" }));

    expect(screen.queryByText("可用变量")).not.toBeInTheDocument();
    expect(screen.queryByText("repository")).not.toBeInTheDocument();

    expect(screen.queryByLabelText("执行指令")).not.toBeInTheDocument();
    expect(
      screen.queryByLabelText("模拟执行耗时 (ms)"),
    ).not.toBeInTheDocument();
  });
});
