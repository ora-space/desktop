import { describe, expect, it } from "vitest";
import {
  createMockWorkflowCapabilities,
  createMockWorkflowNode,
} from "../src";

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
            executor: { agentCli: "code_agent_cli", modelId: "gpt-5" },
            roleId: "Architect",
            skills: [],
            prompt: "",
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
          configFields: ["instruction"],
        },
        {
          kind: "prompt",
          label: "提示词",
          description: "处理和转换文本",
          configFields: ["model", "instruction"],
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
        { agentCli: "code_agent_cli", modelId: "gpt-5", label: "CodeAgentCLI · GPT-5" },
        { agentCli: "open_code", modelId: "opencode/sonnet", label: "OpenCode · Sonnet" },
        { agentCli: "nga", modelId: "nga/default", label: "NGA · Default" },
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
        { value: "openspec-apply-change", label: "openspec-apply-change" },
        { value: "openspec-archive-change", label: "openspec-archive-change" },
        { value: "openspec-bulk-archive-change", label: "openspec-bulk-archive-change" },
        { value: "openspec-continue-change", label: "openspec-continue-change" },
        { value: "openspec-explore", label: "openspec-explore" },
        { value: "openspec-ff-change", label: "openspec-ff-change" },
        { value: "openspec-new-change", label: "openspec-new-change" },
        { value: "openspec-onboard", label: "openspec-onboard" },
        { value: "openspec-propose", label: "openspec-propose" },
        { value: "openspec-sync-specs", label: "openspec-sync-specs" },
        { value: "openspec-verify-change", label: "openspec-verify-change" },
      ],
      tools: [
        { value: "Terminal", label: "Terminal" },
        { value: "File system", label: "File system" },
        { value: "GitHub", label: "GitHub" },
      ],
      defaultModel: "GPT-5",
      defaultAgentConfig: {
        schemaVersion: 3,
        executor: { agentCli: "code_agent_cli", modelId: "gpt-5" },
        roleId: "Architect",
        skills: [],
        prompt: "",
      },
      defaultTool: "Terminal",
    });
  });
});
