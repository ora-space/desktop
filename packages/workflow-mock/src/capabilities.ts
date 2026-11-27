import { DEMO_AGENT_REF } from "./agent-identity";
import type { WorkflowAgentConfig, WorkflowNodeKind } from "./node-data";

export interface WorkflowChoice {
  value: string;
  label: string;
}

/** Availability is descriptive: authors can save bindings before configuring the plugin. */
export interface WorkflowMcpChoice extends WorkflowChoice {
  unavailableReason?:
    | "configurationIncomplete"
    | "configurationUnavailable"
    | "invalidDeclaration";
}

export type WorkflowConfigField =
  | "agent"
  | "initialPrompt"
  | "tool"
  | "condition"
  | "aggregation"
  | "approvalPrompt"
  | "waitStrategy"
  | "failureStrategy"
  | "maxAttempts"
  | "exitCondition"
  | "maxIterations"
  | "maxIterationsBehavior"
  | "loopInitialValue"
  | "iteration";

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
  /** Resolves persisted legacy references against the complete runtime catalog. */
  resolveSkill?: (reference: string) => WorkflowChoice & {
    status: "installed" | "unavailable" | "missing" | "ambiguous";
    catalogId?: string;
    invocationName?: string;
  };
  /** MCP catalog choices for Agent node attachments (optional per node). */
  mcps: WorkflowMcpChoice[];
  tools: WorkflowChoice[];
  /** Comparison operators offered by Condition nodes, keyed by stable value. */
  conditionOperators: WorkflowChoice[];
  /** Operations offered per tool value, mirroring Dify's tool-derived actions. */
  toolOperations: Record<string, WorkflowChoice[]>;
  defaultModel: string;
  defaultAgentConfig: WorkflowAgentConfig;
  defaultTool: string;
}

export interface WorkflowNodeType {
  kind: WorkflowNodeKind;
  label: string;
  description: string;
  configFields: WorkflowConfigField[];
  /** Editor scopes in which this node kind can be authored. */
  supportedScopes: WorkflowNodeScope[];
}

export type WorkflowNodeScope = "workflow" | "iteration" | "loop";

/** Returns whether a capability declaration allows a node in the requested editor scope. */
export function supportsWorkflowNodeScope(
  nodeType: WorkflowNodeType,
  scope: WorkflowNodeScope,
): boolean {
  return nodeType.supportedScopes.includes(scope);
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
    createMockWorkflowNodeType("aggregator", locale),
    createMockWorkflowNodeType("iteration", locale),
    createMockWorkflowNodeType("loop", locale),
    createMockWorkflowNodeType("output", locale),
    createMockWorkflowNodeType("loopExit", locale),
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
      value: "starts_with",
      label: locale === "zh-CN" ? "开头是" : "Starts with",
    },
    { value: "ends_with", label: locale === "zh-CN" ? "结尾是" : "Ends with" },
    {
      value: "greater_than",
      label: locale === "zh-CN" ? "大于" : "Greater than",
    },
    { value: "less_than", label: locale === "zh-CN" ? "小于" : "Less than" },
    {
      value: "greater_than_or_equal",
      label: locale === "zh-CN" ? "大于等于" : "Greater than or equal",
    },
    {
      value: "less_than_or_equal",
      label: locale === "zh-CN" ? "小于等于" : "Less than or equal",
    },
    { value: "empty", label: locale === "zh-CN" ? "为空" : "Is empty" },
    {
      value: "not_empty",
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
        configFields: ["initialPrompt"],
        supportedScopes: ["workflow"],
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
        supportedScopes: ["workflow", "iteration", "loop"],
      };
    case "condition":
      return {
        kind,
        label: locale === "zh-CN" ? "条件分支" : "Condition",
        description:
          locale === "zh-CN"
            ? "根据规则选择路径"
            : "Route execution based on rules",
        configFields: ["condition"],
        supportedScopes: ["workflow", "iteration", "loop"],
      };
    case "aggregator":
      return {
        kind,
        label: locale === "zh-CN" ? "变量聚合器" : "Variable Aggregator",
        description:
          locale === "zh-CN"
            ? "将多分支变量聚合为一个变量，按声明顺序取第一个已产出的值"
            : "Collapse multi-branch variables into one, passing through the first produced value in declaration order",
        configFields: ["aggregation"],
        supportedScopes: ["workflow", "iteration"],
      };
    case "tool":
      return {
        kind,
        label: locale === "zh-CN" ? "工具" : "Tool",
        description:
          locale === "zh-CN" ? "调用终端或插件" : "Call a terminal or plugin",
        configFields: ["tool"],
        supportedScopes: ["workflow"],
      };
    case "junction":
      return {
        kind,
        label: locale === "zh-CN" ? "汇合" : "Merge",
        description:
          locale === "zh-CN"
            ? "等待多个执行分支完成"
            : "Wait for multiple branches to complete",
        configFields: ["waitStrategy", "failureStrategy"],
        supportedScopes: ["workflow"],
      };
    case "human":
      return {
        kind,
        label: locale === "zh-CN" ? "人工确认" : "Human confirmation",
        description:
          locale === "zh-CN"
            ? "等待人工决策后继续"
            : "Pause for a human decision",
        configFields: ["approvalPrompt"],
        supportedScopes: ["workflow"],
      };
    case "loopExit":
      return {
        kind,
        label: locale === "zh-CN" ? "退出循环" : "Exit loop",
        description:
          locale === "zh-CN"
            ? "结束当前循环并继续外部流程"
            : "Exit this loop and continue the outer workflow",
        configFields: [],
        supportedScopes: ["loop"],
      };
    case "loop":
      return {
        kind,
        label: locale === "zh-CN" ? "循环" : "Loop",
        description:
          locale === "zh-CN"
            ? "重复执行直到满足条件"
            : "Repeat until the exit condition is met",
        configFields: [
          "maxIterations",
          "maxIterationsBehavior",
          "loopInitialValue",
        ],
        supportedScopes: ["workflow"],
      };
    case "iteration":
      return {
        kind,
        label: locale === "zh-CN" ? "迭代" : "Iteration",
        description:
          locale === "zh-CN"
            ? "对数组逐项执行区域内节点"
            : "Run the region once per array element",
        configFields: ["iteration"],
        supportedScopes: ["workflow"],
      };
    case "subflow":
      return {
        kind,
        label: locale === "zh-CN" ? "子流程" : "Subflow",
        description:
          locale === "zh-CN"
            ? "封装复杂业务步骤"
            : "Encapsulate a complex business step",
        configFields: [],
        supportedScopes: ["workflow"],
      };
    case "output":
      return {
        kind,
        label: locale === "zh-CN" ? "输出" : "Output",
        description:
          locale === "zh-CN" ? "返回最终结果" : "Return the final result",
        configFields: [],
        supportedScopes: ["workflow", "loop"],
      };
  }
}
