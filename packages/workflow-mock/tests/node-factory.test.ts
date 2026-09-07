import { DEMO_AGENT_REF } from "../src/agent-identity";
import { describe, expect, it } from "vitest";
import { createMockWorkflowCapabilities, createMockWorkflowNode } from "../src";

describe("createMockWorkflowNode", () => {
  it("keeps localized prototype defaults inside the mock package", () => {
    expect([
      createMockWorkflowNode({
        kind: "agent",
        sequence: 2,
        position: { x: 120, y: 240 },
        locale: "zh-CN",
      }),
      createMockWorkflowNode({
        kind: "condition",
        sequence: 3,
        position: { x: 360, y: 240 },
        locale: "en-US",
      }),
    ]).toEqual([
      {
        id: "agent-2",
        type: "workflow",
        position: { x: 120, y: 240 },
        data: {
          kind: "agent",
          title: "Agent 2",
          description: "交给模型自主执行",
          agentConfig: {
            schemaVersion: 3,
            executor: {
              agentCli: DEMO_AGENT_REF.codeagentcli,
              modelId: "gpt-5",
            },
            roleId: "Architect",
            skills: [],
            mcps: [],
            prompt: "",
            interactive: false,
          },
        },
      },
      {
        id: "condition-3",
        type: "workflow",
        position: { x: 360, y: 240 },
        data: {
          kind: "condition",
          title: "Condition 3",
          description: "Route execution based on rules",
          condition: "Condition is met",
          cases: [{ id: "case-1", logic: "and", conditions: [] }],
        },
      },
    ]);
  });

  it("creates Output nodes without an execution instruction", () => {
    expect(
      createMockWorkflowNode({
        kind: "output",
        sequence: 4,
        position: { x: 480, y: 240 },
        locale: "zh-CN",
      }),
    ).toEqual({
      id: "output-4",
      type: "workflow",
      position: { x: 480, y: 240 },
      data: {
        kind: "output",
        title: "输出 4",
        description: "返回最终结果",
      },
    });
  });

  it("provides localized model and tool capabilities for the inspector", () => {
    expect(createMockWorkflowCapabilities("zh-CN")).toEqual({
      nodeTypes: [
        {
          kind: "start",
          label: "开始",
          description: "定义工作流输入",
          configFields: ["initialPrompt"],
        },
        {
          kind: "agent",
          label: "Agent",
          description: "交给模型自主执行",
          configFields: ["agent"],
        },
        {
          kind: "condition",
          label: "条件分支",
          description: "根据规则选择路径",
          configFields: ["condition"],
        },
        {
          kind: "output",
          label: "输出",
          description: "返回最终结果",
          configFields: [],
        },
      ],
      models: [
        { value: "GPT-5", label: "GPT-5" },
        { value: "Claude Sonnet 4", label: "Claude Sonnet 4" },
        { value: "Local model", label: "本地模型" },
      ],
      agentModels: [
        {
          agentCli: DEMO_AGENT_REF.codeagentcli,
          modelId: "gpt-5",
          label: "CodeAgentCLI · GPT-5",
        },
        {
          agentCli: DEMO_AGENT_REF.opencode,
          modelId: "opencode/sonnet",
          label: "OpenCode · Sonnet",
        },
        {
          agentCli: DEMO_AGENT_REF.opencode,
          modelId: "deepseek/deepseek-v4-flash",
          label: "OpenCode · deepseek/deepseek-v4-flash",
        },
        {
          agentCli: DEMO_AGENT_REF.opencode,
          modelId: "deepseek/deepseek-v4-pro",
          label: "OpenCode · deepseek/deepseek-v4-pro",
        },
        {
          agentCli: DEMO_AGENT_REF.nga,
          modelId: "nga/default",
          label: "NGA · Default",
        },
      ],
      roles: [
        { value: "Architect", label: "架构师" },
        { value: "Planner", label: "规划师" },
        { value: "Researcher", label: "研究员" },
        { value: "Implementer", label: "实施者" },
        { value: "Reviewer", label: "审查员" },
        { value: "Tester", label: "测试员" },
        { value: "Debugger", label: "调试员" },
        { value: "Documentation Agent", label: "文档专员" },
      ],
      skills: [
        { value: "cdase:sfmea_review", label: "cdase:sfmea_review" },
        { value: "code-defect-scan", label: "code-defect-scan" },
      ],
      mcps: [
        { value: "filesystem", label: "Filesystem" },
        { value: "github", label: "GitHub" },
        { value: "browser", label: "Browser" },
        { value: "postgres", label: "Postgres" },
        { value: "notion", label: "Notion" },
      ],
      tools: [
        { value: "Terminal", label: "Terminal" },
        { value: "File system", label: "File system" },
        { value: "GitHub", label: "GitHub" },
      ],
      conditionOperators: [
        { value: "equals", label: "等于" },
        { value: "not_equals", label: "不等于" },
        { value: "contains", label: "包含" },
        { value: "not_contains", label: "不包含" },
        { value: "greater_than", label: "大于" },
        { value: "less_than", label: "小于" },
        { value: "empty", label: "为空" },
        { value: "not_empty", label: "不为空" },
      ],
      toolOperations: {
        Terminal: [{ value: "run_command", label: "执行命令" }],
        "File system": [
          { value: "read_file", label: "读取文件" },
          { value: "write_file", label: "写入文件" },
        ],
        GitHub: [
          { value: "create_pr", label: "创建 Pull Request" },
          { value: "merge_pr", label: "合并 Pull Request" },
        ],
      },
      defaultModel: "GPT-5",
      defaultAgentConfig: {
        schemaVersion: 3,
        executor: { agentCli: DEMO_AGENT_REF.codeagentcli, modelId: "gpt-5" },
        roleId: "Architect",
        skills: [],
        mcps: [],
        prompt: "",
        interactive: false,
      },
      defaultTool: "Terminal",
    });
  });
});
