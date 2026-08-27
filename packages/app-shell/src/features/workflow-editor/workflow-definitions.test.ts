import { describe, expect, it } from "vitest";
import { nextWorkflowCopyName } from "./workflow-definitions";

describe("nextWorkflowCopyName", () => {
  it("chains localized suffixes until the name is unused", () => {
    expect(
      nextWorkflowCopyName("Workflow", (name) => `${name} - copy`, [
        "workflow",
        "WORKFLOW - COPY",
      ]),
    ).toBe("Workflow - copy - copy");
  });

  it("rejects a copy-name template that cycles through existing names", () => {
    expect(() =>
      nextWorkflowCopyName("Workflow", () => "Workflow", ["Workflow"]),
    ).toThrow("Copy name generator did not produce a unique workflow name.");
  });
});
