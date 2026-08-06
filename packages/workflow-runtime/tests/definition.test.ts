import { describe, expect, it } from "vitest";
import { createMockWorkflow } from "@ora/workflow-mock";
import {
  normalizeWorkflowDefinition,
  validateWorkflowDefinition,
  WorkflowDefinitionValidationError,
} from "../src/index";

describe("workflow definition validation", () => {
  it("accepts a normalized executable DAG", () => {
    const definition = normalizeWorkflowDefinition(createMockWorkflow("en-US"));

    expect(() => validateWorkflowDefinition(definition)).not.toThrow();
  });

  it("rejects cycles before they can leave a run permanently running", () => {
    const definition = normalizeWorkflowDefinition(createMockWorkflow("en-US"));
    const firstNode = definition.nodes[0]!;
    const lastNode = definition.nodes.at(-1)!;
    definition.edges.push({
      id: "cycle",
      source: lastNode.id,
      target: firstNode.id,
    });

    expect(() => validateWorkflowDefinition(definition)).toThrowError(
      expect.objectContaining<Partial<WorkflowDefinitionValidationError>>({
        name: "WorkflowDefinitionValidationError",
        issues: expect.arrayContaining(["graph must be acyclic"]),
      }),
    );
  });

  it("reports duplicate ids and dangling edges at the deploy boundary", () => {
    const definition = normalizeWorkflowDefinition(createMockWorkflow("en-US"));
    definition.nodes.push(structuredClone(definition.nodes[0]!));
    definition.edges.push({
      id: "dangling",
      source: definition.nodes[0]!.id,
      target: "missing-node",
    });

    expect(() => validateWorkflowDefinition(definition)).toThrowError(
      expect.objectContaining<Partial<WorkflowDefinitionValidationError>>({
        issues: expect.arrayContaining([
          expect.stringContaining("duplicate node id"),
          expect.stringContaining("references an unknown node"),
        ]),
      }),
    );
  });
});
