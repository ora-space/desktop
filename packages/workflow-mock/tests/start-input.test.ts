import { describe, expect, it } from "vitest";
import {
  resolveWorkflowInputFieldType,
  workflowInputFieldValueType,
  WORKFLOW_INPUT_FIELD_TYPES,
} from "../src";

describe("Start input field types", () => {
  it("maps every supported field control to its variable-pool type", () => {
    expect(
      Object.fromEntries(
        WORKFLOW_INPUT_FIELD_TYPES.map((fieldType) => [
          fieldType,
          workflowInputFieldValueType(fieldType),
        ]),
      ),
    ).toEqual({
      "text-input": "string",
      paragraph: "string",
      select: "string",
      number: "number",
      checkbox: "boolean",
      file: "file",
      "file-list": "array[file]",
      json: "object",
    });
  });

  it("derives controls for legacy declarations", () => {
    expect(resolveWorkflowInputFieldType({ valueType: "integer" })).toBe(
      "number",
    );
    expect(resolveWorkflowInputFieldType({ valueType: "array[file]" })).toBe(
      "file-list",
    );
    expect(resolveWorkflowInputFieldType({ valueType: "object" })).toBe("json");
  });
});
