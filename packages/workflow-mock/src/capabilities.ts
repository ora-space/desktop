import type { WorkflowNodeKind } from "./node-data";

export interface WorkflowChoice {
  value: string;
  label: string;
}

export type WorkflowConfigField = "model" | "tool" | "condition" | "instruction";

export interface WorkflowCapabilities {
  nodeTypes: WorkflowNodeType[];
  models: WorkflowChoice[];
  tools: WorkflowChoice[];
  defaultModel: string;
  defaultTool: string;
}

export interface WorkflowNodeType {
  kind: WorkflowNodeKind;
  label: string;
  description: string;
  configFields: WorkflowConfigField[];
}

/** Returns localized prototype capabilities that a real workflow backend can replace later. */
export function createMockWorkflowCapabilities(
  locale: "zh-CN" | "en-US",
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
  return {
    nodeTypes,
    models,
    tools,
    defaultModel: models[0].value,
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
        configFields: ["model", "instruction"],
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
