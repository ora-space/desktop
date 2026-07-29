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
  labelKey: string;
  descriptionKey: string;
  icon: ComponentType<IconProps>;
  tone: string;
}

export const WORKFLOW_NODE_CATALOG: readonly WorkflowNodeMetadata[] = [
  {
    kind: "start",
    labelKey: "settings.workflow.kind.start",
    descriptionKey: "settings.workflow.kind.startDescription",
    icon: IconPlayerPlay,
    tone: "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400",
  },
  {
    kind: "prompt",
    labelKey: "settings.workflow.kind.prompt",
    descriptionKey: "settings.workflow.kind.promptDescription",
    icon: IconMessageCode,
    tone: "bg-violet-500/12 text-violet-700 dark:text-violet-400",
  },
  {
    kind: "agent",
    labelKey: "settings.workflow.kind.agent",
    descriptionKey: "settings.workflow.kind.agentDescription",
    icon: IconBolt,
    tone: "bg-blue-500/12 text-blue-700 dark:text-blue-400",
  },
  {
    kind: "condition",
    labelKey: "settings.workflow.kind.condition",
    descriptionKey: "settings.workflow.kind.conditionDescription",
    icon: IconBinaryTree,
    tone: "bg-amber-500/12 text-amber-700 dark:text-amber-400",
  },
  {
    kind: "tool",
    labelKey: "settings.workflow.kind.tool",
    descriptionKey: "settings.workflow.kind.toolDescription",
    icon: IconBraces,
    tone: "bg-cyan-500/12 text-cyan-700 dark:text-cyan-400",
  },
  {
    kind: "output",
    labelKey: "settings.workflow.kind.output",
    descriptionKey: "settings.workflow.kind.outputDescription",
    icon: IconArrowRight,
    tone: "bg-rose-500/12 text-rose-700 dark:text-rose-400",
  },
];

/** Resolves stable visual metadata for nodes loaded from mock or future backend data. */
export function getNodeMetadata(kind: WorkflowNodeKind): WorkflowNodeMetadata {
  return WORKFLOW_NODE_CATALOG.find((item) => item.kind === kind) ?? WORKFLOW_NODE_CATALOG[0];
}
