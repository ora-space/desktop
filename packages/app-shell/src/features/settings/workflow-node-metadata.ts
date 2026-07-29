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
  icon: ComponentType<IconProps>;
  tone: string;
}

const WORKFLOW_NODE_METADATA: Record<WorkflowNodeKind, WorkflowNodeMetadata> = {
  start: {
    kind: "start",
    icon: IconPlayerPlay,
    tone: "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400",
  },
  prompt: {
    kind: "prompt",
    icon: IconMessageCode,
    tone: "bg-violet-500/12 text-violet-700 dark:text-violet-400",
  },
  agent: {
    kind: "agent",
    icon: IconBolt,
    tone: "bg-blue-500/12 text-blue-700 dark:text-blue-400",
  },
  condition: {
    kind: "condition",
    icon: IconBinaryTree,
    tone: "bg-amber-500/12 text-amber-700 dark:text-amber-400",
  },
  tool: {
    kind: "tool",
    icon: IconBraces,
    tone: "bg-cyan-500/12 text-cyan-700 dark:text-cyan-400",
  },
  output: {
    kind: "output",
    icon: IconArrowRight,
    tone: "bg-rose-500/12 text-rose-700 dark:text-rose-400",
  },
};

/** Resolves stable visual metadata for nodes loaded from mock or future backend data. */
export function getNodeMetadata(kind: WorkflowNodeKind): WorkflowNodeMetadata {
  return WORKFLOW_NODE_METADATA[kind];
}
