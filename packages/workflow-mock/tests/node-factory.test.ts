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
        kind: "agent",
        title: "Agent 2",
        description: "交给模型自主执行",
        position: { x: 120, y: 240 },
        config: { instruction: "", model: "GPT-5" },
      },
      {
        id: "condition-3",
        kind: "condition",
        title: "Condition 3",
        description: "Route execution based on rules",
        position: { x: 360, y: 240 },
        config: { instruction: "", condition: "Condition is met" },
      },
    ]);
  });

  it("provides localized model and tool capabilities for the inspector", () => {
    expect(createMockWorkflowCapabilities("zh-CN")).toEqual({
      nodeTypes: [
        { kind: "start", label: "开始", description: "定义工作流输入" },
        { kind: "prompt", label: "提示词", description: "处理和转换文本" },
        { kind: "agent", label: "Agent", description: "交给模型自主执行" },
        { kind: "condition", label: "条件分支", description: "根据规则选择路径" },
        { kind: "tool", label: "工具", description: "调用终端或插件" },
        { kind: "output", label: "输出", description: "返回最终结果" },
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
