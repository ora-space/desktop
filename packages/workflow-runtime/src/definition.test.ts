import { describe, expect, it } from "vitest";
import {
  normalizeWorkflowDocument,
  validateWorkflowDefinition,
} from "./definition";
import { parseWorkflowGraph, serializeWorkflowGraph } from "./graph-codec";
import type { WorkflowAgentConfig } from "./types";

/** Builds a complete interactive agent config; the case varies only the completion policy. */
function agentConfig(
  overrides: Partial<WorkflowAgentConfig> = {},
): WorkflowAgentConfig {
  return {
    schemaVersion: 3,
    executor: {
      agentCli: "official/ora-space.opencode",
      modelId: "anthropic/claude-sonnet-4",
    },
    roleId: "",
    skills: [],
    mcps: [],
    prompt: "review",
    interactive: true,
    ...overrides,
  };
}

describe("authoring document normalization", () => {
  it("preserves spare cycles through persistence while executable validation remains strict", () => {
    const definition = normalizeWorkflowDocument({
      id: "workflow",
      name: "Spare nodes",
      description: "",
      updatedAt: "2026-09-20T12:00:00+08:00",
      viewport: { x: 0, y: 0, zoom: 1 },
      nodes: [
        {
          id: "start",
          position: { x: 0, y: 0 },
          data: { kind: "start", title: "Start", description: "" },
        },
        {
          id: "spare",
          position: { x: 0, y: 100 },
          data: { kind: "agent", title: "Spare", description: "" },
        },
      ],
      edges: [{ id: "cycle", source: "spare", target: "spare" }],
    });
    const restored = parseWorkflowGraph(serializeWorkflowGraph(definition));
    expect({ nodes: restored.nodes, edges: restored.edges }).toEqual({
      nodes: definition.nodes,
      edges: definition.edges,
    });
    expect(() => validateWorkflowDefinition(definition)).toThrow(
      "graph must be acyclic",
    );
  });

  it("saves an interactive agent on the explicit completion signal by default", () => {
    const definition = normalizeWorkflowDocument({
      id: "workflow",
      name: "Defaults",
      description: "",
      updatedAt: "2026-09-20T12:00:00+08:00",
      viewport: { x: 0, y: 0, zoom: 1 },
      nodes: [
        {
          id: "agent",
          position: { x: 0, y: 0 },
          data: {
            kind: "agent",
            title: "Review",
            description: "",
            agentConfig: agentConfig(),
          },
        },
        {
          id: "legacy",
          position: { x: 20, y: 0 },
          data: {
            kind: "agent",
            title: "Legacy",
            description: "",
            agentConfig: agentConfig({ completionSignal: "end_turn" }),
          },
        },
      ],
      edges: [],
    });
    const [agent, legacy] = definition.nodes;
    expect(agent?.data.agentConfig).toMatchObject({
      interactive: true,
      completionSignal: "agent",
      schemaVersion: 4,
    });
    expect(legacy?.data.agentConfig).toMatchObject({
      interactive: true,
      completionSignal: "end_turn",
    });
  });
});
