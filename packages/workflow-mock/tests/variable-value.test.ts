import { describe, expect, it } from "vitest";
import {
  normalizeWorkflowVariableValue,
  parseWorkflowVariableValueText,
  WORKFLOW_VARIABLE_VALUE_TYPES,
  workflowVariableValueExample,
  workflowVariableValueMatchesType,
  type WorkflowVariableValueType,
} from "../src";

describe("workflow variable values", () => {
  it("accepts one representative value for every declared type", () => {
    const cases: [WorkflowVariableValueType, unknown][] = [
      ["string", "text"],
      ["number", 1.5],
      ["integer", 1],
      ["boolean", true],
      ["secret", "hidden"],
      ["file", { kind: "workspace_file", path: "docs/input.txt" }],
      ["object", { key: "value" }],
      ["any", null],
      ["array", [1, "two", false]],
      ["array[string]", ["one", "two"]],
      ["array[number]", [1, 2.5]],
      ["array[object]", [{ one: 1 }, { two: 2 }]],
      ["array[boolean]", [true, false]],
      ["array[file]", [{ kind: "workspace_file", path: "one.txt" }]],
      ["array[any]", [1, "two", null]],
    ];

    for (const [valueType, value] of cases) {
      expect(
        workflowVariableValueMatchesType(value, valueType),
        valueType,
      ).toBe(true);
    }
  });

  it("distinguishes constrained arrays from untyped arrays", () => {
    const mixed = [1, "two", { three: 3 }];
    expect(workflowVariableValueMatchesType(mixed, "array")).toBe(true);
    expect(workflowVariableValueMatchesType(mixed, "array[any]")).toBe(true);
    expect(workflowVariableValueMatchesType(mixed, "array[string]")).toBe(
      false,
    );
    expect(workflowVariableValueMatchesType(mixed, "array[object]")).toBe(
      false,
    );
  });

  it("normalizes legacy paths and rejects unsafe file references", () => {
    expect(normalizeWorkflowVariableValue("docs\\input.txt", "file")).toEqual({
      valid: true,
      value: { kind: "workspace_file", path: "docs/input.txt" },
    });
    expect(
      parseWorkflowVariableValueText(
        '["one.txt","nested/two.txt"]',
        "array[file]",
      ),
    ).toEqual({
      valid: true,
      value: [
        { kind: "workspace_file", path: "one.txt" },
        { kind: "workspace_file", path: "nested/two.txt" },
      ],
    });
    expect(normalizeWorkflowVariableValue("../secret", "file")).toEqual({
      valid: false,
      issue: "invalid_file",
    });
  });

  it("reports invalid typed editor input without coercing it to a string", () => {
    expect(parseWorkflowVariableValueText("1.5", "integer")).toEqual({
      valid: false,
      issue: "invalid_type",
    });
    expect(parseWorkflowVariableValueText("yes", "boolean")).toEqual({
      valid: false,
      issue: "invalid_boolean",
    });
    expect(
      parseWorkflowVariableValueText('["ok", 2]', "array[string]"),
    ).toEqual({
      valid: false,
      issue: "invalid_type",
    });
  });

  it("provides a parseable example for every declared type", () => {
    for (const valueType of WORKFLOW_VARIABLE_VALUE_TYPES) {
      expect(
        parseWorkflowVariableValueText(
          workflowVariableValueExample(valueType),
          valueType,
        ).valid,
        valueType,
      ).toBe(true);
    }
  });
});
