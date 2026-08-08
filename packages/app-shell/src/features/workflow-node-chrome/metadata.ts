import type { ComponentType } from "react";
import {
  IconArrowRight,
  IconBinaryTree,
  IconBolt,
  IconBraces,
  IconGitMerge,
  IconHierarchy2,
  IconPlayerPlay,
  IconRepeat,
  IconUserCheck,
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
  junction: {
    kind: "junction",
    icon: IconGitMerge,
    tone: "bg-teal-500/12 text-teal-700 dark:text-teal-400",
  },
  human: {
    kind: "human",
    icon: IconUserCheck,
    tone: "bg-sky-500/12 text-sky-700 dark:text-sky-400",
  },
  loop: {
    kind: "loop",
    icon: IconRepeat,
    tone: "bg-indigo-500/12 text-indigo-700 dark:text-indigo-400",
  },
  subflow: {
    kind: "subflow",
    icon: IconHierarchy2,
    tone: "bg-fuchsia-500/12 text-fuchsia-700 dark:text-fuchsia-400",
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
