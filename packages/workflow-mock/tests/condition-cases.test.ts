import { describe, expect, it } from "vitest";
import {
  isWorkflowConditionComparisonComplete,
  resolveConditionCases,
  type WorkflowNodeData,
} from "../src";

describe("resolveConditionCases", () => {
  it("returns authored cases unchanged", () => {
    const data: WorkflowNodeData = {
      kind: "condition",
      title: "",
      description: "",
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
    expect(resolveConditionCases(data)).toEqual(data.cases);
  });

  it("migrates legacy branch rules into canonical executable cases", () => {
    const data: WorkflowNodeData = {
      kind: "condition",
      title: "",
      description: "",
      conditionBranches: [
        {
          logic: "and",
          conditions: [
            {
              variable: "review.structured_output.approved",
              operator: "is_empty",
              negated: true,
              value: "",
            },
          ],
        },
      ],
    };

    expect(resolveConditionCases(data)).toEqual([
      {
        id: "case-1",
        logic: "and",
        conditions: [
          {
            variableSelector: ["review", "structured_output", "approved"],
            operator: "not_empty",
          },
        ],
      },
    ]);
  });

  it("defaults to one empty IF case when nothing is authored", () => {
    const data: WorkflowNodeData = {
      kind: "condition",
      title: "",
      description: "",
    };
    expect(resolveConditionCases(data)).toEqual([
      { id: "case-1", logic: "and", conditions: [] },
    ]);
  });

  it("keeps an explicitly empty case list empty", () => {
    const data: WorkflowNodeData = {
      kind: "condition",
      title: "",
      description: "",
      cases: [],
    };
    expect(resolveConditionCases(data)).toEqual([]);
  });
});

describe("isWorkflowConditionComparisonComplete", () => {
  it("keeps partially authored value comparisons unset", () => {
    expect(
      isWorkflowConditionComparisonComplete({
        variableSelector: [],
        operator: "equals",
      }),
    ).toBe(false);
    expect(
      isWorkflowConditionComparisonComplete({
        variableSelector: ["review", "text"],
        operator: "equals",
        value: "",
      }),
    ).toBe(false);
    expect(
      isWorkflowConditionComparisonComplete({
        variableSelector: ["review", "text"],
        operator: "",
        value: "ready",
      }),
    ).toBe(false);
  });

  it("accepts zero, false, and valueless operators as complete values", () => {
    expect(
      isWorkflowConditionComparisonComplete({
        variableSelector: ["review", "score"],
        operator: "equals",
        value: 0,
      }),
    ).toBe(true);
    expect(
      isWorkflowConditionComparisonComplete({
        variableSelector: ["review", "approved"],
        operator: "is",
        value: false,
      }),
    ).toBe(true);
    expect(
      isWorkflowConditionComparisonComplete({
        variableSelector: ["review", "text"],
        operator: "empty",
      }),
    ).toBe(true);
  });
});
