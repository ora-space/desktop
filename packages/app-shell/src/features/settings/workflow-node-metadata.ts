import type { ComponentType } from "react";
import {
  IconArrowRight,
  IconBinaryTree,
  IconBraces,
  IconCode,
  IconDatabase,
  IconRobot,
  IconTemplate,
  IconWebhook,
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

export interface WorkflowNodeGroup {
  labelKey: string;
  items: readonly WorkflowNodeMetadata[];
}

export const WORKFLOW_NODE_DRAG_DATA_TYPE = "application/x-ora-workflow-node";

export const WORKFLOW_NODE_GROUPS: readonly WorkflowNodeGroup[] = [
  {
    labelKey: "settings.workflow.group.input",
    items: [
      {
        kind: "trigger",
        labelKey: "settings.workflow.kind.trigger",
        descriptionKey: "settings.workflow.kind.triggerDescription",
        icon: IconWebhook,
        tone: "bg-emerald-500/12 text-emerald-700 dark:text-emerald-400",
      },
      {
        kind: "data-source",
        labelKey: "settings.workflow.kind.dataSource",
        descriptionKey: "settings.workflow.kind.dataSourceDescription",
        icon: IconDatabase,
        tone: "bg-orange-500/12 text-orange-700 dark:text-orange-400",
      },
    ],
  },
  {
    labelKey: "settings.workflow.group.process",
    items: [
      {
        kind: "llm",
        labelKey: "settings.workflow.kind.llm",
        descriptionKey: "settings.workflow.kind.llmDescription",
        icon: IconRobot,
        tone: "bg-blue-500/12 text-blue-700 dark:text-blue-400",
      },
      {
        kind: "code",
        labelKey: "settings.workflow.kind.code",
        descriptionKey: "settings.workflow.kind.codeDescription",
        icon: IconCode,
        tone: "bg-cyan-500/12 text-cyan-700 dark:text-cyan-400",
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
        tone: "bg-indigo-500/12 text-indigo-700 dark:text-indigo-400",
      },
      {
        kind: "template",
        labelKey: "settings.workflow.kind.template",
        descriptionKey: "settings.workflow.kind.templateDescription",
        icon: IconTemplate,
        tone: "bg-violet-500/12 text-violet-700 dark:text-violet-400",
      },
    ],
  },
  {
    labelKey: "settings.workflow.group.output",
    items: [
      {
        kind: "output",
        labelKey: "settings.workflow.kind.output",
        descriptionKey: "settings.workflow.kind.outputDescription",
        icon: IconArrowRight,
        tone: "bg-rose-500/12 text-rose-700 dark:text-rose-400",
      },
    ],
  },
];

export const WORKFLOW_NODE_CATALOG: readonly WorkflowNodeMetadata[] =
  WORKFLOW_NODE_GROUPS.flatMap((group) => group.items);

/** Resolves stable visual metadata for nodes loaded from mock or future backend data. */
export function getNodeMetadata(kind: WorkflowNodeKind): WorkflowNodeMetadata {
  return WORKFLOW_NODE_CATALOG.find((item) => item.kind === kind) ?? WORKFLOW_NODE_CATALOG[0];
}
