import { isEdge, isNode } from "@xyflow/react";
import {
  WORKFLOW_ANNOTATION_THEMES,
  type WorkflowAnnotationNode,
} from "./annotation-data";
import { WORKFLOW_NODE_KINDS, type WorkflowAgentConfig } from "./node-data";
import type { DemoWorkflow } from "./fixtures";

/** Validates imported React Flow elements before they enter the canvas. */
export function isDemoWorkflow(value: unknown): value is DemoWorkflow {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const candidate = value as Partial<DemoWorkflow>;
  if (!(
    typeof candidate.id === "string" &&
    candidate.id.trim() !== "" &&
    typeof candidate.name === "string" &&
    candidate.name.trim() !== "" &&
    typeof candidate.description === "string" &&
    typeof candidate.updatedAt === "string" &&
    typeof candidate.viewport === "object" &&
    candidate.viewport !== null &&
    Number.isFinite(candidate.viewport.x) &&
    Number.isFinite(candidate.viewport.y) &&
    Number.isFinite(candidate.viewport.zoom) &&
    candidate.viewport.zoom > 0 &&
    Array.isArray(candidate.nodes) &&
    Array.isArray(candidate.edges) &&
    (candidate.annotations === undefined ||
      (Array.isArray(candidate.annotations) &&
        candidate.annotations.every(isWorkflowAnnotation))) &&
    candidate.nodes.every(
      (node) =>
        isNode(node) &&
        typeof node.id === "string" &&
        node.id.trim() !== "" &&
        node.type === "workflow" &&
        Number.isFinite(node.position?.x) &&
        Number.isFinite(node.position?.y) &&
        typeof node.data === "object" &&
        node.data !== null &&
        WORKFLOW_NODE_KINDS.includes(node.data.kind) &&
        (node.data.kind !== "start" || node.deletable === false) &&
        typeof node.data.title === "string" &&
        typeof node.data.description === "string" &&
        (node.data.kind === "agent"
          ? isWorkflowAgentConfig(node.data.agentConfig)
          : typeof node.data.instruction === "string"),
    ) &&
    candidate.edges.every(
      (edge) =>
        isEdge(edge) &&
        typeof edge.id === "string" &&
        edge.id.trim() !== "" &&
        edge.type === "workflow" &&
        typeof edge.source === "string" &&
        typeof edge.target === "string" &&
        (edge.sourceHandle === undefined || edge.sourceHandle === null) &&
        (edge.targetHandle === undefined || edge.targetHandle === null),
    )
  )) {
    return false;
  }

  const nodeIds = new Set(candidate.nodes.map((node) => node.id));
  const edgeIds = new Set(candidate.edges.map((edge) => edge.id));
  const annotationIds = new Set(
    (candidate.annotations ?? []).map((annotation) => annotation.id),
  );
  return (
    nodeIds.size === candidate.nodes.length &&
    edgeIds.size === candidate.edges.length &&
    annotationIds.size === (candidate.annotations?.length ?? 0) &&
    new Set([...nodeIds, ...edgeIds, ...annotationIds]).size ===
      candidate.nodes.length +
        candidate.edges.length +
        (candidate.annotations?.length ?? 0) &&
    candidate.nodes.filter((node) => node.data.kind === "start").length === 1 &&
    candidate.edges.every(
      (edge) =>
        edge.source !== edge.target &&
        nodeIds.has(edge.source) &&
        nodeIds.has(edge.target),
    ) &&
    new Set(candidate.edges.map((edge) => `${edge.source}\u0000${edge.target}`))
      .size === candidate.edges.length
  );
}

/** Validates editor-only notes without allowing them into executable node validation. */
function isWorkflowAnnotation(value: unknown): value is WorkflowAnnotationNode {
  if (!isNode(value) || value.type !== "annotation") {
    return false;
  }
  return (
    typeof value.id === "string" &&
    value.id.trim() !== "" &&
    Number.isFinite(value.position?.x) &&
    Number.isFinite(value.position?.y) &&
    typeof value.data?.text === "string" &&
    WORKFLOW_ANNOTATION_THEMES.some((theme) => theme === value.data.theme)
  );
}

/** Validates the serialized Agent contract before a definition can be edited or run. */
function isWorkflowAgentConfig(value: unknown): value is WorkflowAgentConfig {
  if (typeof value !== "object" || value === null) {
    return false;
  }
  const config = value as Partial<WorkflowAgentConfig>;
  return (
    config.schemaVersion === 3 &&
    typeof config.executor === "object" &&
    config.executor !== null &&
    typeof config.executor.agentCli === "string" &&
    config.executor.agentCli.trim() !== "" &&
    typeof config.executor.modelId === "string" &&
    config.executor.modelId.trim() !== "" &&
    typeof config.roleId === "string" &&
    config.roleId.trim() !== "" &&
    Array.isArray(config.skills) &&
    config.skills.every(
      (skill) =>
        typeof skill === "object" &&
        skill !== null &&
        typeof skill.skillId === "string" &&
        skill.skillId.trim() !== "" &&
        typeof skill.enabled === "boolean",
    ) &&
    new Set(config.skills.map((skill) => skill.skillId)).size ===
      config.skills.length &&
    Array.isArray(config.mcps) &&
    config.mcps.every(
      (mcp) =>
        typeof mcp === "object" &&
        mcp !== null &&
        typeof mcp.mcpId === "string" &&
        mcp.mcpId.trim() !== "" &&
        typeof mcp.enabled === "boolean",
    ) &&
    new Set(config.mcps.map((mcp) => mcp.mcpId)).size === config.mcps.length &&
    typeof config.prompt === "string"
  );
}
