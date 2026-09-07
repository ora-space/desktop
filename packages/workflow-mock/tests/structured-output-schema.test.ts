import { describe, expect, it } from "vitest";
import { validateWorkflowStructuredOutputSchema } from "../src";

describe("validateWorkflowStructuredOutputSchema", () => {
  it("accepts nested object, file, and typed array fields", () => {
    expect(
      validateWorkflowStructuredOutputSchema({
        type: "object",
        properties: {
          attachment: { type: "file" },
          records: {
            type: "array",
            items: {
              type: "object",
              properties: { score: { type: "number" } },
              required: ["score"],
              additionalProperties: false,
            },
          },
        },
        required: ["attachment"],
        additionalProperties: false,
      }),
    ).toEqual({ valid: true });
  });

  it("rejects unsupported field types", () => {
    expect(
      validateWorkflowStructuredOutputSchema({
        type: "object",
        properties: { date: { type: "date" } },
      }),
    ).toEqual({
      valid: false,
      path: "$.properties.date",
      message: "unsupported type date",
    });
  });

  it("rejects undeclared required fields and misplaced array items", () => {
    expect(
      validateWorkflowStructuredOutputSchema({
        type: "object",
        properties: {},
        required: ["missing"],
      }).valid,
    ).toBe(false);
    expect(
      validateWorkflowStructuredOutputSchema({
        type: "object",
        properties: { value: { type: "string", items: { type: "string" } } },
      }).valid,
    ).toBe(false);
  });
});
