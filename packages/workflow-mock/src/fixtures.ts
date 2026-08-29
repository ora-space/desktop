import type { Edge, Node, ReactFlowJsonObject } from "@xyflow/react";
import type { WorkflowAgentConfig, WorkflowNodeData } from "./node-data";
import type { WorkflowAnnotationNode } from "./annotation-data";
import {
  WORKFLOW_NODE_INITIAL_HANDLES,
  WORKFLOW_NODE_INITIAL_HEIGHT,
  WORKFLOW_NODE_WIDTH,
} from "./node-layout";

export interface DemoWorkflow extends ReactFlowJsonObject<
  Node<WorkflowNodeData, "workflow">,
  Edge
> {
  id: string;
  name: string;
  description: string;
  updatedAt: string;
  /** Editor-only notes are persisted beside, never inside, executable nodes. */
  annotations?: WorkflowAnnotationNode[];
}

/** Builds a stable Agent execution contract for fixtures and future persisted definitions. */
function createAgentConfig(
  roleId: string,
  prompt: string,
  options: {
    skillIds?: string[];
    executor?: WorkflowAgentConfig["executor"];
  } = {},
): WorkflowAgentConfig {
  return {
    schemaVersion: 3,
    executor: options.executor ?? {
      agentCli: "ora-space.codeagentcli",
      modelId: "gpt-5",
    },
    roleId,
    skills: (options.skillIds ?? []).map((skillId) => ({
      skillId,
      enabled: true,
    })),
    mcps: [],
    prompt,
  };
}

export const MOCK_WORKFLOW: DemoWorkflow = {
  id: "code-review",
  name: "代码审查工作流",
  description: "读取改动、执行质量检查，并输出一份可操作的审查摘要。",
  updatedAt: "2026-07-27T11:30:00+08:00",
  viewport: { x: 32, y: 32, zoom: 1 },
  nodes: [
    {
      id: "start",
      type: "workflow",
      deletable: false,
      position: { x: 72, y: 286 },
      data: {
        kind: "start",
        title: "开始",
        description: "接收任务和当前工作区",
        instruction: "从用户输入中提取审查范围。",
        trigger: "merge_request",
        inputVariables: [
          { name: "仓库", defaultValue: "{{repository}}" },
          { name: "目标分支", defaultValue: "{{target_branch}}" },
        ],
      },
    },
    {
      id: "understand",
      type: "workflow",
      position: { x: 356, y: 188 },
      data: {
        kind: "agent",
        title: "理解改动",
        description: "总结变更意图与影响范围",
        agentConfig: createAgentConfig(
          "Planner",
          "阅读改动文件，整理变更目标、受影响模块和潜在风险。",
        ),
      },
    },
    {
      id: "quality",
      type: "workflow",
      position: { x: 650, y: 188 },
      data: {
        kind: "condition",
        title: "质量门禁",
        description: "判断是否需要执行测试",
        instruction: "根据改动类型选择后续路径。",
        conditionBranches: [
          {
            conditions: [
              { variable: "改动类型", operator: "contains", value: "源代码" },
            ],
          },
        ],
      },
    },
    {
      id: "tests",
      type: "workflow",
      position: { x: 938, y: 92 },
      data: {
        kind: "tool",
        title: "运行检查",
        description: "执行格式化、类型检查和测试",
        instruction: "运行与改动范围匹配的最小验证集。",
        tool: "Terminal",
        operation: "run_command",
        toolParameters: [{ key: "command", value: "npm run test" }],
      },
    },
    {
      id: "review",
      type: "workflow",
      position: { x: 938, y: 330 },
      data: {
        kind: "agent",
        title: "审查 Agent",
        description: "综合代码与验证结果",
        agentConfig: createAgentConfig(
          "Reviewer",
          "按严重程度整理问题，并给出定位与修复建议。",
          { skillIds: ["code-defect-scan"] },
        ),
      },
    },
    {
      id: "output",
      type: "workflow",
      position: { x: 1218, y: 330 },
      data: {
        kind: "output",
        title: "输出报告",
        description: "生成结构化审查结论",
        instruction: "输出摘要、发现、验证结果和后续建议。",
      },
    },
  ],
  edges: [
    {
      id: "e-start-understand",
      source: "start",
      target: "understand",
      type: "workflow",
    },
    {
      id: "e-understand-quality",
      source: "understand",
      target: "quality",
      type: "workflow",
    },
    {
      id: "e-quality-tests",
      source: "quality",
      target: "tests",
      type: "workflow",
      label: "需要检查",
    },
    {
      id: "e-quality-review",
      source: "quality",
      target: "review",
      type: "workflow",
      label: "仅文档",
    },
    {
      id: "e-tests-review",
      source: "tests",
      target: "review",
      type: "workflow",
    },
    {
      id: "e-review-output",
      source: "review",
      target: "output",
      type: "workflow",
    },
  ],
};

const ENGLISH_NODE_CONTENT: Record<
  string,
  Pick<
    WorkflowNodeData,
    | "title"
    | "description"
    | "instruction"
    | "trigger"
    | "inputVariables"
    | "tool"
    | "condition"
    | "conditionBranches"
    | "operation"
    | "toolParameters"
  >
> = {
  start: {
    title: "Start",
    description: "Receive the task and current workspace",
    instruction: "Extract the review scope from the user input.",
    trigger: "merge_request",
    inputVariables: [
      { name: "Repository", defaultValue: "{{repository}}" },
      { name: "Target branch", defaultValue: "{{target_branch}}" },
    ],
  },
  understand: {
    title: "Understand changes",
    description: "Summarize intent and affected areas",
  },
  quality: {
    title: "Quality gate",
    description: "Decide whether validation is required",
    instruction: "Choose the next path based on the type of change.",
    conditionBranches: [
      {
        conditions: [
          {
            variable: "Change type",
            operator: "contains",
            value: "source code",
          },
        ],
      },
    ],
  },
  tests: {
    title: "Run checks",
    description: "Run formatting, type checks, and tests",
    instruction:
      "Run the smallest validation set that matches the change scope.",
    tool: "Terminal",
    operation: "run_command",
    toolParameters: [{ key: "command", value: "npm run test" }],
  },
  review: {
    title: "Review agent",
    description: "Evaluate code and validation results",
  },
  output: {
    title: "Output report",
    description: "Generate a structured review result",
    instruction:
      "Return a summary, findings, validation results, and next steps.",
  },
};

/** Per-node Agent prompts for the English fixture, keyed by node id. */
const ENGLISH_AGENT_PROMPTS: Record<string, string> = {
  understand:
    "Read changed files and identify the goal, affected modules, and potential risks.",
  review:
    "Organize findings by severity and provide locations and remediation advice.",
};

/** Creates localized fixture content with native React Flow nodes and edges. */
export function createMockWorkflow(locale: "zh-CN" | "en-US"): DemoWorkflow {
  const workflow = structuredClone(MOCK_WORKFLOW);
  // Predeclared measurements let React Flow resolve demo edges before the
  // custom cards are measured, which also makes server/test rendering stable.
  workflow.nodes = workflow.nodes.map((node) => ({
    ...node,
    initialWidth: WORKFLOW_NODE_WIDTH,
    initialHeight: WORKFLOW_NODE_INITIAL_HEIGHT,
    handles: WORKFLOW_NODE_INITIAL_HANDLES.map((handle) => ({ ...handle })),
  }));
  if (locale === "zh-CN") {
    return workflow;
  }
  workflow.name = "Code review workflow";
  workflow.description =
    "Read changes, run quality checks, and produce an actionable review summary.";
  workflow.nodes = workflow.nodes.map((node) => ({
    ...node,
    data:
      node.data.kind === "agent"
        ? {
            ...node.data,
            ...ENGLISH_NODE_CONTENT[node.id],
            agentConfig: {
              ...node.data.agentConfig!,
              prompt:
                ENGLISH_AGENT_PROMPTS[node.id] ?? node.data.agentConfig!.prompt,
            },
          }
        : { ...node.data, ...ENGLISH_NODE_CONTENT[node.id] },
  }));
  workflow.edges = workflow.edges.map((edge) => ({
    ...edge,
    label:
      edge.label === "需要检查"
        ? "Checks required"
        : edge.label === "仅文档"
          ? "Documentation only"
          : edge.label,
  }));
  return workflow;
}

/** Provides selectable session workflows for the React Flow demo. */
export function createMockWorkflows(locale: "zh-CN" | "en-US"): DemoWorkflow[] {
  const staggered = createStaggeredParallelMockWorkflow(locale);
  const parallel = createParallelMockWorkflow(locale);
  const review = createMockWorkflow(locale);
  const release = structuredClone(review);
  release.id = "release-readiness";
  release.name = locale === "zh-CN" ? "发布准备检查" : "Release readiness";
  release.description =
    locale === "zh-CN"
      ? "在部署前验证测试、变更说明和风险项。"
      : "Validate tests, release notes, and risks before deployment.";
  release.updatedAt = "2026-07-26T16:40:00+08:00";

  const triage = structuredClone(review);
  triage.id = "issue-triage";
  triage.name = locale === "zh-CN" ? "问题分类助手" : "Issue triage assistant";
  triage.description =
    locale === "zh-CN"
      ? "分析问题描述并分配优先级与处理角色。"
      : "Analyze issue reports and assign priority and ownership.";
  triage.updatedAt = "2026-07-25T09:20:00+08:00";

  return [staggered, parallel, review, release, triage];
}

/**
 * Fan-out / fan-in fixture so the run Theater can show multiple live acts.
 * After「收集上下文」, security / quality / docs run concurrently, then synthesize.
 */
export function createParallelMockWorkflow(
  locale: "zh-CN" | "en-US",
): DemoWorkflow {
  const zh = locale === "zh-CN";
  const workflow: DemoWorkflow = {
    id: "parallel-review",
    name: zh ? "并行审查演示" : "Parallel review demo",
    description: zh
      ? "收集上下文后同时跑安全、质量与文档分支，再汇总结论——用于可视化并行舞台。"
      : "After gathering context, security, quality, and docs run together before merge — for testing the parallel Theater.",
    updatedAt: "2026-08-01T16:00:00+08:00",
    viewport: { x: 40, y: 40, zoom: 0.9 },
    nodes: [
      {
        id: "start",
        type: "workflow",
        deletable: false,
        position: { x: 48, y: 280 },
        data: {
          kind: "start",
          title: zh ? "开始" : "Start",
          description: zh ? "接收审查范围" : "Receive review scope",
          instruction: zh
            ? "解析用户输入中的目标路径。"
            : "Parse the target scope from user input.",
        },
      },
      {
        id: "gather",
        type: "workflow",
        position: { x: 320, y: 280 },
        data: {
          kind: "human",
          title: zh ? "收集上下文" : "Gather context",
          description: zh
            ? "汇总改动与相关文件"
            : "Summarize changes and related files",
          instruction: zh
            ? "列出改动文件、模块边界和已知风险点。"
            : "List changed files, module boundaries, and known risks.",
        },
      },
      {
        id: "security",
        type: "workflow",
        position: { x: 620, y: 80 },
        data: {
          kind: "agent",
          title: zh ? "安全审查" : "Security review",
          description: zh ? "并行分支 · 安全" : "Parallel branch · security",
          agentConfig: createAgentConfig(
            "Reviewer",
            zh
              ? "检查注入、权限与密钥泄露风险。"
              : "Check for injection, auth, and secret-leak risks.",
            { skillIds: ["code-defect-scan"] },
          ),
        },
      },
      {
        id: "quality",
        type: "workflow",
        position: { x: 620, y: 280 },
        data: {
          kind: "tool",
          title: zh ? "质量检查" : "Quality checks",
          description: zh ? "并行分支 · 质量" : "Parallel branch · quality",
          instruction: zh
            ? "运行格式化、类型检查与相关测试。"
            : "Run formatting, typecheck, and related tests.",
          tool: "Terminal",
        },
      },
      {
        id: "docs",
        type: "workflow",
        position: { x: 620, y: 480 },
        data: {
          kind: "human",
          title: zh ? "文档一致性" : "Docs consistency",
          description: zh ? "并行分支 · 文档" : "Parallel branch · docs",
          instruction: zh
            ? "核对 README / 注释是否与改动一致。"
            : "Check README / comments against the change set.",
        },
      },
      {
        id: "synthesize",
        type: "workflow",
        position: { x: 920, y: 280 },
        data: {
          kind: "agent",
          title: zh ? "汇总结论" : "Synthesize",
          description: zh
            ? "合并三条并行分支"
            : "Merge the three parallel branches",
          agentConfig: createAgentConfig(
            "Architect",
            zh
              ? "按严重程度合并安全、质量与文档发现。"
              : "Merge security, quality, and docs findings by severity.",
            { skillIds: ["code-defect-scan"] },
          ),
        },
      },
      {
        id: "output",
        type: "workflow",
        position: { x: 1180, y: 280 },
        data: {
          kind: "output",
          title: zh ? "输出报告" : "Output report",
          description: zh ? "生成结构化摘要" : "Produce a structured summary",
          instruction: zh
            ? "输出摘要、发现列表与后续建议。"
            : "Emit summary, findings, and follow-ups.",
        },
      },
    ],
    edges: [
      {
        id: "e-start-gather",
        source: "start",
        target: "gather",
        type: "workflow",
      },
      {
        id: "e-gather-security",
        source: "gather",
        target: "security",
        type: "workflow",
      },
      {
        id: "e-gather-quality",
        source: "gather",
        target: "quality",
        type: "workflow",
      },
      {
        id: "e-gather-docs",
        source: "gather",
        target: "docs",
        type: "workflow",
      },
      {
        id: "e-security-synth",
        source: "security",
        target: "synthesize",
        type: "workflow",
      },
      {
        id: "e-quality-synth",
        source: "quality",
        target: "synthesize",
        type: "workflow",
      },
      {
        id: "e-docs-synth",
        source: "docs",
        target: "synthesize",
        type: "workflow",
      },
      {
        id: "e-synth-output",
        source: "synthesize",
        target: "output",
        type: "workflow",
      },
    ],
  };

  workflow.nodes = workflow.nodes.map((node) => ({
    ...node,
    initialWidth: WORKFLOW_NODE_WIDTH,
    initialHeight: WORKFLOW_NODE_INITIAL_HEIGHT,
    handles: WORKFLOW_NODE_INITIAL_HANDLES.map((handle) => ({ ...handle })),
  }));
  return workflow;
}

/**
 * Staggered fan-out fixture: branches share a start wave but use different
 * `mockStepMs` and unequal depths so starts/ends diverge while overlapping.
 *
 * Timeline intent (ms after start completes):
 * - t0: quick_scan (1.5s), lint (3.5s), slow_index (5.5s) begin together
 * - t1.5: deep_security (6s) begins while lint + slow_index still run
 * - t3.5: lint ends; deep_security + slow_index still live
 * - t5.5: docs_pass (2s) begins after slow_index; may overlap deep_security
 * - join waits until deep_security + lint + docs_pass all finish
 */
export function createStaggeredParallelMockWorkflow(
  locale: "zh-CN" | "en-US",
): DemoWorkflow {
  const zh = locale === "zh-CN";
  const workflow: DemoWorkflow = {
    id: "staggered-parallel",
    name: zh ? "错开并行演示" : "Staggered parallel demo",
    description: zh
      ? "多条分支同时推进，但启动与结束时刻不同——覆盖长短任务交错的并行舞台。"
      : "Several branches overlap with different start and end times — covers staggered parallel Theater.",
    updatedAt: "2026-08-01T17:20:00+08:00",
    viewport: { x: 24, y: 24, zoom: 0.85 },
    nodes: [
      {
        id: "start",
        type: "workflow",
        deletable: false,
        position: { x: 40, y: 300 },
        data: {
          kind: "start",
          title: zh ? "开始" : "Start",
          description: zh ? "接收改动范围" : "Receive change scope",
          instruction: zh
            ? "解析目标仓库与分支。"
            : "Parse target repo and branch.",
          mockStepMs: 800,
        },
      },
      {
        id: "quick_scan",
        type: "workflow",
        position: { x: 300, y: 80 },
        data: {
          kind: "human",
          title: zh ? "快速扫描" : "Quick scan",
          description: zh ? "短任务 · 先结束" : "Short task · finishes first",
          instruction: zh
            ? "做一次廉价的风险预检。"
            : "Run a cheap risk pre-check.",
          mockStepMs: 1_500,
        },
      },
      {
        id: "deep_security",
        type: "workflow",
        position: { x: 560, y: 80 },
        data: {
          kind: "agent",
          title: zh ? "深度安全" : "Deep security",
          description: zh ? "晚启动 · 长耗时" : "Starts later · long-running",
          agentConfig: createAgentConfig(
            "Reviewer",
            zh
              ? "在快速扫描之后做深度安全分析。"
              : "Follow the quick scan with a deep security analysis.",
            { skillIds: ["code-defect-scan"] },
          ),
          mockStepMs: 6_000,
        },
      },
      {
        id: "lint",
        type: "workflow",
        position: { x: 300, y: 300 },
        data: {
          kind: "tool",
          title: zh ? "Lint / 类型" : "Lint / types",
          description: zh ? "中等时长 · 直达汇合" : "Medium · joins directly",
          instruction: zh ? "跑 lint 与类型检查。" : "Run lint and typecheck.",
          tool: "Terminal",
          mockStepMs: 3_500,
        },
      },
      {
        id: "slow_index",
        type: "workflow",
        position: { x: 300, y: 520 },
        data: {
          kind: "tool",
          title: zh ? "索引构建" : "Build index",
          description: zh
            ? "最长前置 · 拖慢下游"
            : "Slowest prep · delays downstream",
          instruction: zh
            ? "重建搜索索引（故意偏慢）。"
            : "Rebuild the search index (intentionally slow).",
          tool: "Terminal",
          mockStepMs: 5_500,
        },
      },
      {
        id: "docs_pass",
        type: "workflow",
        position: { x: 560, y: 520 },
        data: {
          kind: "human",
          title: zh ? "文档校对" : "Docs pass",
          description: zh ? "最晚启动 · 短尾" : "Latest start · short tail",
          instruction: zh
            ? "索引完成后核对文档。"
            : "Proof docs after the index is ready.",
          mockStepMs: 2_000,
        },
      },
      {
        id: "join",
        type: "workflow",
        position: { x: 840, y: 300 },
        data: {
          kind: "junction",
          title: zh ? "汇合" : "Join",
          description: zh
            ? "等待全部交错分支"
            : "Wait for all staggered branches",
          instruction: zh
            ? "合并安全、lint 与文档结果。"
            : "Merge security, lint, and docs results.",
          waitStrategy: "all",
          failureStrategy: "fail",
          mockStepMs: 2_000,
        },
      },
      {
        id: "output",
        type: "workflow",
        position: { x: 1100, y: 300 },
        data: {
          kind: "output",
          title: zh ? "输出" : "Output",
          description: zh ? "写出摘要" : "Write the summary",
          instruction: zh
            ? "输出交错并行演示的结论。"
            : "Emit the staggered-parallel demo conclusion.",
          mockStepMs: 1_000,
        },
      },
    ],
    edges: [
      {
        id: "e-start-scan",
        source: "start",
        target: "quick_scan",
        type: "workflow",
      },
      { id: "e-start-lint", source: "start", target: "lint", type: "workflow" },
      {
        id: "e-start-index",
        source: "start",
        target: "slow_index",
        type: "workflow",
      },
      {
        id: "e-scan-security",
        source: "quick_scan",
        target: "deep_security",
        type: "workflow",
      },
      {
        id: "e-index-docs",
        source: "slow_index",
        target: "docs_pass",
        type: "workflow",
      },
      {
        id: "e-security-join",
        source: "deep_security",
        target: "join",
        type: "workflow",
      },
      { id: "e-lint-join", source: "lint", target: "join", type: "workflow" },
      {
        id: "e-docs-join",
        source: "docs_pass",
        target: "join",
        type: "workflow",
      },
      {
        id: "e-join-output",
        source: "join",
        target: "output",
        type: "workflow",
      },
    ],
  };

  workflow.nodes = workflow.nodes.map((node) => ({
    ...node,
    initialWidth: WORKFLOW_NODE_WIDTH,
    initialHeight: WORKFLOW_NODE_INITIAL_HEIGHT,
    handles: WORKFLOW_NODE_INITIAL_HANDLES.map((handle) => ({ ...handle })),
  }));
  return workflow;
}
