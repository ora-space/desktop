import { describe, expect, it } from "vitest";
import {
  isIteratorSource,
  isCollectOutput,
  supportsFeedback,
  validReferences,
  invalidComparisonField,
  type WorkflowVariableCatalogEntry,
  type WorkflowVariableValueType,
  type WorkflowConditionComparison,
} from "../src/index";

function entry(
  selector: string[],
  valueType: WorkflowVariableValueType,
): WorkflowVariableCatalogEntry {
  return {
    selector,
    sourceNodeId: selector[0]!,
    variableName: selector.slice(1).join("."),
    valueType,
  };
}
describe("iteration field catalogs", () => {
  it.each([
    {
      selector: ["start", "items"],
      type: "array[string]",
      source: true,
      collect: false,
    },
    {
      selector: ["rev", "structured_output", "items"],
      type: "array",
      source: true,
      collect: false,
    },
    {
      selector: ["rev", "structured_output"],
      type: "object",
      source: false,
      collect: true,
    },
    {
      selector: ["rev", "output"],
      type: "string",
      source: false,
      collect: true,
    },
    {
      selector: ["rev", "nested", "output"],
      type: "string",
      source: false,
      collect: false,
    },
  ] as const)("classifies $selector", ({ selector, type, source, collect }) => {
    const variable = entry([...selector], type);
    expect([isIteratorSource(variable), isCollectOutput(variable)]).toEqual([
      source,
      collect,
    ]);
  });
});
describe("feedback declaration compatibility", () => {
  it.each([
    ["boolean", "boolean", true],
    ["integer", "number", false],
    ["number", "integer", true],
    ["number", "string", false],
    ["string", "secret", true],
    ["secret", "string", true],
    ["any", "boolean", true],
    ["boolean", "any", true],
    ["object", "file", true],
    ["array", "array[file]", true],
    ["array[object]", "array[file]", true],
    ["array[string]", "array[file]", false],
    ["array[any]", "array[number]", true],
    ["array[number]", "array[string]", false],
    ["array[string]", "array", false],
  ] as const)("%s accepts %s: %s", (target, source, expected) =>
    expect(supportsFeedback(target, source)).toBe(expected),
  );
});

const catalog = [
  entry(["rev", "structured_output", "count"], "number"),
  entry(["rev", "name"], "string"),
];
it.each([
  ["literal", true],
  ["{{#rev.name#}} {{#rev.structured_output.count#}}", true],
  ["{{#rev.structured_output#}}", false],
  ["{{#rev.name", false],
  ["{{#rev.name#}} {{#unknown.name#}}", false],
] as const)("checks scoped tokens in %s", (value, expected) =>
  expect(validReferences(value, catalog)).toBe(expected),
);

it.each([
  { operator: "greater_than", value: 1, field: undefined },
  {
    operator: "greater_than",
    value: "{{#rev.structured_output.count#}}",
    field: undefined,
  },
  { operator: "greater_than", value: "{{#rev.name#}}", field: "value" },
  {
    operator: "greater_than",
    value: "score {{#rev.structured_output.count#}}",
    field: "value",
  },
  { operator: "contains", value: "x", field: "operator" },
  { operator: "unknown", value: 1, field: "operator" },
  { operator: "exists", field: undefined },
  { operator: "is_not", value: 0, field: undefined },
])("checks $operator and its operand", ({ operator, value, field }) => {
  const rule: WorkflowConditionComparison = {
    variableSelector: catalog[0]!.selector,
    operator,
    value,
  };
  const choices = ["greater_than", "contains"].map((value) => ({
    value,
    label: value,
  }));
  expect(invalidComparisonField(rule, catalog, choices)).toBe(field);
  expect(invalidComparisonField(rule, [], choices)).toBe("variable");
});
