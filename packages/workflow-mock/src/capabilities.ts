import { DEMO_AGENT_REF } from "./agent-identity";
import type { WorkflowAgentConfig, WorkflowNodeKind } from "./node-data";

export interface WorkflowChoice {
  value: string;
  label: string;
}

export type WorkflowConfigField =
  | "agent"
  | "trigger"
  | "tool"
  | "condition"
  | "instruction"
  | "waitStrategy"
  | "failureStrategy"
  | "maxAttempts"
  | "exitCondition";

export interface WorkflowAgentModel {
  agentCli: string;
  modelId: string;
  label: string;
}

export interface WorkflowCapabilities {
  nodeTypes: WorkflowNodeType[];
  models: WorkflowChoice[];
  agentModels: WorkflowAgentModel[];
  roles: WorkflowChoice[];
  skills: WorkflowChoice[];
  /** MCP catalog choices for Agent node attachments (optional per node). */
  mcps: WorkflowChoice[];
  tools: WorkflowChoice[];
  /** Comparison operators offered by Condition nodes, keyed by stable value. */
  conditionOperators: WorkflowChoice[];
  /** Operations offered per tool value, mirroring Dify's tool-derived actions. */
  toolOperations: Record<string, WorkflowChoice[]>;
  /** Trigger methods offered by Start nodes (merge request, push, manual). */
  startTriggers: WorkflowChoice[];
  defaultTrigger: string;
  defaultModel: string;
  defaultAgentConfig: WorkflowAgentConfig;
  defaultTool: string;
}

/** Context variables every workflow run exposes, referenced as `{{name}}` in node configs. */
export const WORKFLOW_CONTEXT_VARIABLES = [
  "repository",
  "branch",
  "commit_id",
  "mr_id",
  "changed_files",
] as const;

export interface WorkflowNodeType {
  kind: WorkflowNodeKind;
  label: string;
  description: string;
  configFields: WorkflowConfigField[];
}

const DEFAULT_AGENT_MODEL: WorkflowAgentModel = {
  agentCli: DEMO_AGENT_REF.codeagentcli,
  modelId: "gpt-5",
  label: "CodeAgentCLI · GPT-5",
};

const MOCK_AGENT_MODELS: WorkflowAgentModel[] = [
  DEFAULT_AGENT_MODEL,
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
];

const MOCK_AGENT_ROLES: WorkflowChoice[] = [
  { value: "Architect", label: "架构师" },
  { value: "Planner", label: "规划师" },
  { value: "Researcher", label: "研究员" },
  { value: "Implementer", label: "实施者" },
  { value: "Reviewer", label: "审查员" },
  { value: "Tester", label: "测试员" },
  { value: "Debugger", label: "调试员" },
  { value: "Documentation Agent", label: "文档专员" },
];

const MOCK_AGENT_SKILLS: WorkflowChoice[] = [
  "cdase:sfmea_review",
  "code-defect-scan",
].map((value) => ({ value, label: value }));

const MOCK_AGENT_MCPS: WorkflowChoice[] = [
  { value: "filesystem", label: "Filesystem" },
  { value: "github", label: "GitHub" },
  { value: "browser", label: "Browser" },
  { value: "postgres", label: "Postgres" },
  { value: "notion", label: "Notion" },
];

/**
 * Returns prototype workflow capabilities, optionally using models discovered
 * by the backend while retaining local Role and Skill catalogs until their
 * backend APIs are available.
 */
export function createMockWorkflowCapabilities(
  locale: "zh-CN" | "en-US",
  agentModels: WorkflowAgentModel[] = MOCK_AGENT_MODELS,
): WorkflowCapabilities {
  const nodeTypes: WorkflowNodeType[] = [
    createMockWorkflowNodeType("start", locale),
    createMockWorkflowNodeType("agent", locale),
    createMockWorkflowNodeType("condition", locale),
    createMockWorkflowNodeType("tool", locale),
    createMockWorkflowNodeType("junction", locale),
    createMockWorkflowNodeType("human", locale),
    createMockWorkflowNodeType("loop", locale),
    createMockWorkflowNodeType("subflow", locale),
    createMockWorkflowNodeType("output", locale),
  ];
  const models = [
    { value: "GPT-5", label: "GPT-5" },
    { value: "Claude Sonnet 4", label: "Claude Sonnet 4" },
    {
      value: "Local model",
      label: locale === "zh-CN" ? "本地模型" : "Local model",
    },
  ];
  const tools = [
    { value: "Terminal", label: "Terminal" },
    { value: "File system", label: "File system" },
    { value: "GitHub", label: "GitHub" },
  ];
  const conditionOperators = [
    { value: "equals", label: locale === "zh-CN" ? "等于" : "Equals" },
    {
      value: "not_equals",
      label: locale === "zh-CN" ? "不等于" : "Not equals",
    },
    { value: "contains", label: locale === "zh-CN" ? "包含" : "Contains" },
    {
      value: "not_contains",
      label: locale === "zh-CN" ? "不包含" : "Not contains",
    },
    {
      value: "greater_than",
      label: locale === "zh-CN" ? "大于" : "Greater than",
    },
    { value: "less_than", label: locale === "zh-CN" ? "小于" : "Less than" },
    { value: "is_empty", label: locale === "zh-CN" ? "为空" : "Is empty" },
    {
      value: "is_not_empty",
      label: locale === "zh-CN" ? "不为空" : "Is not empty",
    },
  ];
  const toolOperations = {
    Terminal: [
      {
        value: "run_command",
        label: locale === "zh-CN" ? "执行命令" : "Run command",
      },
    ],
    "File system": [
      {
        value: "read_file",
        label: locale === "zh-CN" ? "读取文件" : "Read file",
      },
      {
        value: "write_file",
        label: locale === "zh-CN" ? "写入文件" : "Write file",
      },
    ],
    GitHub: [
      {
        value: "create_pr",
        label: locale === "zh-CN" ? "创建 Pull Request" : "Create pull request",
      },
      {
        value: "merge_pr",
        label: locale === "zh-CN" ? "合并 Pull Request" : "Merge pull request",
      },
    ],
  } satisfies Record<string, WorkflowChoice[]>;
  const startTriggers = [
    { value: "merge_request", label: "Merge Request" },
    { value: "push", label: "Push" },
    { value: "manual", label: locale === "zh-CN" ? "手动" : "Manual" },
  ];
  const defaultAgentModel = agentModels[0] ?? DEFAULT_AGENT_MODEL;
  return {
    nodeTypes,
    models,
    agentModels,
    roles: MOCK_AGENT_ROLES,
    skills: MOCK_AGENT_SKILLS,
    mcps: MOCK_AGENT_MCPS,
    tools,
    conditionOperators,
    toolOperations,
    startTriggers,
    defaultTrigger: startTriggers[0]!.value,
    defaultModel: models[0].value,
    defaultAgentConfig: {
      schemaVersion: 3,
      executor: {
        agentCli: defaultAgentModel.agentCli,
        modelId: defaultAgentModel.modelId,
      },
      roleId: MOCK_AGENT_ROLES[0]!.value,
      skills: [],
      mcps: [],
      prompt: "",
      interactive: false,
      outputPolicy: "none",
    },
    defaultTool: tools[0].value,
  };
}

/** Resolves localized mock content for one supported workflow node kind. */
export function createMockWorkflowNodeType(
  kind: WorkflowNodeKind,
  locale: "zh-CN" | "en-US",
): WorkflowNodeType {
  switch (kind) {
    case "start":
      return {
        kind,
        label: locale === "zh-CN" ? "开始" : "Start",
        description:
          locale === "zh-CN" ? "定义工作流输入" : "Define workflow inputs",
        configFields: ["instruction", "trigger"],
      };
    case "agent":
      return {
        kind,
        label: "Agent",
        description:
          locale === "zh-CN"
            ? "交给模型自主执行"
            : "Delegate autonomous work to a model",
        configFields: ["agent"],
      };
    case "condition":
      return {
        kind,
        label: locale === "zh-CN" ? "条件分支" : "Condition",
        description:
          locale === "zh-CN"
            ? "根据规则选择路径"
            : "Route execution based on rules",
        configFields: ["condition", "instruction"],
      };
    case "tool":
      return {
        kind,
        label: locale === "zh-CN" ? "工具" : "Tool",
        description:
          locale === "zh-CN" ? "调用终端或插件" : "Call a terminal or plugin",
        configFields: ["tool", "instruction"],
      };
    case "junction":
      return {
        kind,
        label: locale === "zh-CN" ? "汇合" : "Merge",
        description:
          locale === "zh-CN"
            ? "等待多个执行分支完成"
            : "Wait for multiple branches to complete",
        configFields: ["instruction", "waitStrategy", "failureStrategy"],
      };
    case "human":
      return {
        kind,
        label: locale === "zh-CN" ? "人工确认" : "Human confirmation",
        description:
          locale === "zh-CN"
            ? "等待人工决策后继续"
            : "Pause for a human decision",
        configFields: ["instruction"],
      };
    case "loop":
      return {
        kind,
        label: locale === "zh-CN" ? "循环" : "Loop",
        description:
          locale === "zh-CN"
            ? "重复执行直到满足条件"
            : "Repeat until the exit condition is met",
        configFields: ["instruction", "maxAttempts", "exitCondition"],
      };
    case "subflow":
      return {
        kind,
        label: locale === "zh-CN" ? "子流程" : "Subflow",
        description:
          locale === "zh-CN"
            ? "封装复杂业务步骤"
            : "Encapsulate a complex business step",
        configFields: ["instruction"],
      };
    case "output":
      return {
        kind,
        label: locale === "zh-CN" ? "输出" : "Output",
        description:
          locale === "zh-CN" ? "返回最终结果" : "Return the final result",
        configFields: ["instruction"],
      };
  }
}
