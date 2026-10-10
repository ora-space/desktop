// Pure translation data: safe to compose without importing feature implementation.
export const workflowNodeTranslations = {
  "zh-CN": {
    "workflowNode.unused": "未参与运行",
    "workflowNode.unusedHint":
      "此节点或所属容器未从作用域入口连通，不会参与运行。",
    "workflowNode.unusedCount": "有 {{count}} 个节点未参与运行",
    "workflowNode.unrecognizedCount":
      "有 {{count}} 个节点的类型无法识别（{{kinds}}）。它们不是有意保留的备用节点，加载时会被丢弃，也不会参与运行。",
    "workflowNode.agentExecutionMode.interactive": "人工交互节点",
    "workflowNode.agentExecutionMode.automatic": "自动执行节点",
  },
  "en-US": {
    "workflowNode.unused": "Excluded from execution",
    "workflowNode.unusedHint":
      "This node or its container is not reachable from its scope entry and will not run.",
    "workflowNode.unusedCount": "{{count}} nodes excluded from execution",
    "workflowNode.unrecognizedCount":
      "{{count}} nodes use an unrecognized type ({{kinds}}). They are not spare nodes kept on purpose; loading drops them, and they will not run.",
    "workflowNode.agentExecutionMode.interactive": "Interactive Agent node",
    "workflowNode.agentExecutionMode.automatic": "Automatic Agent node",
  },
} as const;
