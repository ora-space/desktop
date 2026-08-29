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
            executor: { agentCli: "ora-space.codeagentcli", modelId: "gpt-5" },
            roleId: "Architect",
            skills: [],
            mcps: [],
            prompt: "",
            interactive: false,
            outputPolicy: "none",
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
          instruction: "",
          condition: "Condition is met",
        },
      },
    ]);
  });

  it("provides localized model and tool capabilities for the inspector", () => {
    expect(createMockWorkflowCapabilities("zh-CN")).toEqual({
      nodeTypes: [
        {
          kind: "start",
          label: "开始",
          description: "定义工作流输入",
          configFields: ["instruction", "trigger"],
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
          configFields: ["condition", "instruction"],
        },
        {
          kind: "tool",
          label: "工具",
          description: "调用终端或插件",
          configFields: ["tool", "instruction"],
        },
        {
          kind: "junction",
          label: "汇合",
          description: "等待多个执行分支完成",
          configFields: ["instruction", "waitStrategy", "failureStrategy"],
        },
        {
          kind: "human",
          label: "人工确认",
          description: "等待人工决策后继续",
          configFields: ["instruction"],
        },
        {
          kind: "loop",
          label: "循环",
          description: "重复执行直到满足条件",
          configFields: ["instruction", "maxAttempts", "exitCondition"],
        },
        {
          kind: "subflow",
          label: "子流程",
          description: "封装复杂业务步骤",
          configFields: ["instruction"],
        },
        {
          kind: "output",
          label: "输出",
          description: "返回最终结果",
          configFields: ["instruction"],
        },
      ],
      models: [
        { value: "GPT-5", label: "GPT-5" },
        { value: "Claude Sonnet 4", label: "Claude Sonnet 4" },
        { value: "Local model", label: "本地模型" },
      ],
      agentModels: [
        {
          agentCli: "ora-space.codeagentcli",
          modelId: "gpt-5",
          label: "CodeAgentCLI · GPT-5",
        },
        {
          agentCli: "ora-space.opencode",
          modelId: "opencode/sonnet",
          label: "OpenCode · Sonnet",
        },
        {
          agentCli: "ora-space.opencode",
          modelId: "deepseek/deepseek-v4-flash",
          label: "OpenCode · deepseek/deepseek-v4-flash",
        },
        {
          agentCli: "ora-space.opencode",
          modelId: "deepseek/deepseek-v4-pro",
          label: "OpenCode · deepseek/deepseek-v4-pro",
        },
        {
          agentCli: "ora-space.nga",
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
        { value: "is_empty", label: "为空" },
        { value: "is_not_empty", label: "不为空" },
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
      startTriggers: [
        { value: "merge_request", label: "Merge Request" },
        { value: "push", label: "Push" },
        { value: "manual", label: "手动" },
      ],
      defaultTrigger: "merge_request",
      defaultModel: "GPT-5",
      defaultAgentConfig: {
        schemaVersion: 3,
        executor: { agentCli: "ora-space.codeagentcli", modelId: "gpt-5" },
        roleId: "Architect",
        skills: [],
        mcps: [],
        prompt: "",
        interactive: false,
        outputPolicy: "none",
      },
      defaultTool: "Terminal",
    });
  });
});
