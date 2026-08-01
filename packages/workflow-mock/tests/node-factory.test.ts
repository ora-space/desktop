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
          instruction: "",
          model: "GPT-5",
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
          configFields: ["model", "instruction"],
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
      tools: [
        { value: "Terminal", label: "Terminal" },
        { value: "File system", label: "File system" },
        { value: "GitHub", label: "GitHub" },
      ],
      defaultModel: "GPT-5",
      defaultTool: "Terminal",
    });
  });
});
