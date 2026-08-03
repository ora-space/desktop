import type {
  WorkflowAgentConfig,
  WorkflowNodeKind,
} from "./node-data";

export interface WorkflowChoice {
  value: string;
  label: string;
}

export type WorkflowConfigField = "agent" | "model" | "tool" | "condition" | "instruction";

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
  tools: WorkflowChoice[];
  defaultModel: string;
  defaultAgentConfig: WorkflowAgentConfig;
  defaultTool: string;
}

export interface WorkflowNodeType {
  kind: WorkflowNodeKind;
  label: string;
  description: string;
  configFields: WorkflowConfigField[];
}

const DEFAULT_AGENT_MODEL: WorkflowAgentModel = {
  agentCli: "code_agent_cli",
  modelId: "gpt-5",
  label: "CodeAgentCLI · GPT-5",
};

const MOCK_AGENT_MODELS: WorkflowAgentModel[] = [
  DEFAULT_AGENT_MODEL,
  { agentCli: "open_code", modelId: "opencode/sonnet", label: "OpenCode · Sonnet" },
  { agentCli: "nga", modelId: "nga/default", label: "NGA · Default" },
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
  "openspec-apply-change",
  "openspec-archive-change",
  "openspec-bulk-archive-change",
  "openspec-continue-change",
  "openspec-explore",
  "openspec-ff-change",
  "openspec-new-change",
  "openspec-onboard",
  "openspec-propose",
  "openspec-sync-specs",
  "openspec-verify-change",
].map((value) => ({ value, label: value }));

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
    createMockWorkflowNodeType("prompt", locale),
    createMockWorkflowNodeType("agent", locale),
    createMockWorkflowNodeType("condition", locale),
    createMockWorkflowNodeType("tool", locale),
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
  const defaultAgentModel = agentModels[0] ?? DEFAULT_AGENT_MODEL;
  return {
    nodeTypes,
    models,
    agentModels,
    roles: MOCK_AGENT_ROLES,
    skills: MOCK_AGENT_SKILLS,
    tools,
    defaultModel: models[0].value,
    defaultAgentConfig: {
      schemaVersion: 3,
      executor: {
        agentCli: defaultAgentModel.agentCli,
        modelId: defaultAgentModel.modelId,
      },
      roleId: MOCK_AGENT_ROLES[0]!.value,
      skills: [],
      prompt: "",
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
        description: locale === "zh-CN" ? "定义工作流输入" : "Define workflow inputs",
        configFields: ["instruction"],
      };
    case "prompt":
      return {
        kind,
        label: locale === "zh-CN" ? "提示词" : "Prompt",
        description: locale === "zh-CN" ? "处理和转换文本" : "Process and transform text",
        configFields: ["model", "instruction"],
      };
    case "agent":
      return {
        kind,
        label: "Agent",
        description: locale === "zh-CN"
          ? "交给模型自主执行"
          : "Delegate autonomous work to a model",
        configFields: ["agent"],
      };
    case "condition":
      return {
        kind,
        label: locale === "zh-CN" ? "条件分支" : "Condition",
        description: locale === "zh-CN"
          ? "根据规则选择路径"
          : "Route execution based on rules",
        configFields: ["condition", "instruction"],
      };
    case "tool":
      return {
        kind,
        label: locale === "zh-CN" ? "工具" : "Tool",
        description: locale === "zh-CN" ? "调用终端或插件" : "Call a terminal or plugin",
        configFields: ["tool", "instruction"],
      };
    case "output":
      return {
        kind,
        label: locale === "zh-CN" ? "输出" : "Output",
        description: locale === "zh-CN" ? "返回最终结果" : "Return the final result",
        configFields: ["instruction"],
      };
  }
}
