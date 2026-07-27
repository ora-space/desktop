import type { ComponentType } from "react";
import {
  IconArrowRight,
  IconBinaryTree,
  IconBolt,
  IconBraces,
  IconMessageCode,
  IconPlayerPlay,
  type IconProps,
} from "@tabler/icons-react";
import type { WorkflowNodeKind } from "@ora/workflow-mock";

export interface WorkflowNodeMetadata {
  kind: WorkflowNodeKind;
  label: string;
  description: string;
  icon: ComponentType<IconProps>;
  tone: string;
}

export const WORKFLOW_NODE_CATALOG: readonly WorkflowNodeMetadata[] = [
  {
    kind: "start",
    label: "开始",
    description: "定义工作流输入",
    icon: IconPlayerPlay,
    tone: "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400",
  },
  {
    kind: "prompt",
    label: "提示词",
    description: "处理和转换文本",
    icon: IconMessageCode,
    tone: "bg-violet-500/12 text-violet-700 dark:text-violet-400",
  },
  {
    kind: "agent",
    label: "Agent",
    description: "交给模型自主执行",
    icon: IconBolt,
    tone: "bg-blue-500/12 text-blue-700 dark:text-blue-400",
  },
  {
    kind: "condition",
    label: "条件分支",
    description: "根据规则选择路径",
    icon: IconBinaryTree,
    tone: "bg-amber-500/12 text-amber-700 dark:text-amber-400",
  },
  {
    kind: "tool",
    label: "工具",
    description: "调用终端或插件",
    icon: IconBraces,
    tone: "bg-cyan-500/12 text-cyan-700 dark:text-cyan-400",
  },
  {
    kind: "output",
    label: "输出",
    description: "返回最终结果",
    icon: IconArrowRight,
    tone: "bg-rose-500/12 text-rose-700 dark:text-rose-400",
  },
];

/** Resolves stable visual metadata for nodes loaded from mock or future backend data. */
export function getNodeMetadata(kind: WorkflowNodeKind): WorkflowNodeMetadata {
  return WORKFLOW_NODE_CATALOG.find((item) => item.kind === kind) ?? WORKFLOW_NODE_CATALOG[0];
}
