import type { WorkflowDefinition, WorkflowLocale } from "./types";

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

const ENGLISH_NODE_CONTENT: Record<
  string,
  Pick<WorkflowDefinition["nodes"][number], "title" | "description" | "config">
> = {
  start: {
    title: "Start",
    description: "Receive the task and current workspace",
    config: { instruction: "Extract the review scope from the user input." },
  },
  understand: {
    title: "Understand changes",
    description: "Summarize intent and affected areas",
    config: {
      instruction: "Read changed files and identify the goal, affected modules, and potential risks.",
      model: "GPT-5",
    },
  },
  quality: {
    title: "Quality gate",
    description: "Decide whether validation is required",
    config: {
      instruction: "Choose the next path based on the type of change.",
      condition: "Contains source code changes",
    },
  },
  tests: {
    title: "Run checks",
    description: "Run formatting, type checks, and tests",
    config: {
      instruction: "Run the smallest validation set that matches the change scope.",
      tool: "Terminal",
    },
  },
  review: {
    title: "Review agent",
    description: "Evaluate code and validation results",
    config: {
      instruction: "Organize findings by severity and provide locations and remediation advice.",
      model: "GPT-5",
    },
  },
  output: {
    title: "Output report",
    description: "Generate a structured review result",
    config: { instruction: "Return a summary, findings, validation results, and next steps." },
  },
};

/** Creates localized fixture content while preserving stable graph identifiers and positions. */
export function createMockWorkflow(locale: WorkflowLocale): WorkflowDefinition {
  const workflow = structuredClone(MOCK_WORKFLOW);
  if (locale === "zh-CN") {
    return workflow;
  }
  workflow.name = "Code review workflow";
  workflow.description = "Read changes, run quality checks, and produce an actionable review summary.";
  workflow.nodes = workflow.nodes.map((node) => ({ ...node, ...ENGLISH_NODE_CONTENT[node.id] }));
  workflow.edges = workflow.edges.map((edge) => ({
    ...edge,
    label: edge.label === "需要检查"
      ? "Checks required"
      : edge.label === "仅文档"
        ? "Documentation only"
        : edge.label,
  }));
  return workflow;
}
