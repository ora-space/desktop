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

  it("migrates a legacy Start instruction to the Start input", () => {
    const workflow = createMockWorkflow("en-US");
    const start = workflow.nodes.find((node) => node.data.kind === "start");
    if (start === undefined) {
      throw new Error("Mock workflow must contain a Start node");
    }
    const legacyStartData = { ...start.data };
    delete legacyStartData.input;
    start.data = { ...legacyStartData, instruction: "Legacy kickoff" };

    const definition = normalizeWorkflowDefinition(workflow);
    const normalizedStart = definition.nodes.find(
      (node) => node.data.kind === "start",
    );

    expect(normalizedStart?.data.input).toBe("Legacy kickoff");
    expect(normalizedStart?.data.instruction).toBeUndefined();
  });

  it("preserves condition source handles and executable cases through normalization", () => {
    const definition = normalizeWorkflowDefinition(createMockWorkflow("en-US"));
    definition.edges[0] = {
      ...definition.edges[0]!,
      sourceHandle: "else",
      targetHandle: undefined,
    };
    const condition = definition.nodes.find(
      (node) => node.data.kind === "condition",
    );
    if (condition !== undefined) {
      condition.data = {
        ...condition.data,
        cases: [
          {
            id: "approved",
            logic: "and",
            conditions: [
              {
                variableSelector: ["review", "structured_output", "approved"],
                operator: "is",
                value: true,
              },
            ],
          },
        ],
      };
    }

    const normalized = normalizeWorkflowDefinition(definition);
    expect(normalized.edges[0]!.sourceHandle).toBe("else");
    const normalizedCondition = normalized.nodes.find(
      (node) => node.data.kind === "condition",
    );
    expect(normalizedCondition?.data.cases).toEqual([
      {
        id: "approved",
        logic: "and",
        conditions: [
          {
            variableSelector: ["review", "structured_output", "approved"],
            operator: "is",
            value: true,
          },
        ],
      },
    ]);
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
