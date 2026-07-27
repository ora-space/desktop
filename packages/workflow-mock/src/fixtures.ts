import type { WorkflowDefinition } from "./types";

export const MOCK_WORKFLOW: WorkflowDefinition = {
  id: "code-review",
  name: "代码审查工作流",
  description: "读取改动、执行质量检查，并输出一份可操作的审查摘要。",
  updatedAt: "2026-07-27T11:30:00+08:00",
  nodes: [
    {
      id: "start",
      kind: "start",
      title: "开始",
      description: "接收任务和当前工作区",
      position: { x: 72, y: 286 },
      config: { instruction: "从用户输入中提取审查范围。" },
    },
    {
      id: "understand",
      kind: "prompt",
      title: "理解改动",
      description: "总结变更意图与影响范围",
      position: { x: 356, y: 188 },
      config: {
        instruction: "阅读改动文件，整理变更目标、受影响模块和潜在风险。",
        model: "GPT-5",
      },
    },
    {
      id: "quality",
      kind: "condition",
      title: "质量门禁",
      description: "判断是否需要执行测试",
      position: { x: 650, y: 188 },
      config: {
        instruction: "根据改动类型选择后续路径。",
        condition: "包含源代码改动",
      },
    },
    {
      id: "tests",
      kind: "tool",
      title: "运行检查",
      description: "执行格式化、类型检查和测试",
      position: { x: 938, y: 92 },
      config: {
        instruction: "运行与改动范围匹配的最小验证集。",
        tool: "Terminal",
      },
    },
    {
      id: "review",
      kind: "agent",
      title: "审查 Agent",
      description: "综合代码与验证结果",
      position: { x: 938, y: 330 },
      config: {
        instruction: "按严重程度整理问题，并给出定位与修复建议。",
        model: "GPT-5",
      },
    },
    {
      id: "output",
      kind: "output",
      title: "输出报告",
      description: "生成结构化审查结论",
      position: { x: 1218, y: 330 },
      config: { instruction: "输出摘要、发现、验证结果和后续建议。" },
    },
  ],
  edges: [
    { id: "e-start-understand", source: "start", target: "understand" },
    { id: "e-understand-quality", source: "understand", target: "quality" },
    { id: "e-quality-tests", source: "quality", target: "tests", label: "需要检查" },
    { id: "e-quality-review", source: "quality", target: "review", label: "仅文档" },
    { id: "e-tests-review", source: "tests", target: "review" },
    { id: "e-review-output", source: "review", target: "output" },
  ],
};
