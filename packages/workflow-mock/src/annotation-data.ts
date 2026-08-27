import type { Node } from "@xyflow/react";

export const WORKFLOW_ANNOTATION_THEMES = [
  "yellow",
  "blue",
  "green",
  "pink",
  "gray",
] as const;

export type WorkflowAnnotationTheme =
  (typeof WORKFLOW_ANNOTATION_THEMES)[number];

/** Stores editor-only note content without entering the executable workflow contract. */
export interface WorkflowAnnotationData extends Record<string, unknown> {
  text: string;
  theme: WorkflowAnnotationTheme;
}

export type WorkflowAnnotationNode = Node<WorkflowAnnotationData, "annotation">;
