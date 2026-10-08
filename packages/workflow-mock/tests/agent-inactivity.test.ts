import { describe, expect, it } from "vitest";
import { createMockWorkflow, parseDemoWorkflow } from "../src";

describe("Agent prompt inactivity configuration", () => {
  it.each([undefined, null, "timeout", "wait"])(
    "imports and preserves the accepted policy %s",
    (promptInactivity) => {
      const workflow = createMockWorkflow("en-US");
      const config = workflow.nodes.find((node) => node.data.kind === "agent")!
        .data.agentConfig!;
      if (promptInactivity !== undefined) {
        Object.assign(config, { promptInactivity });
      }

      const imported = parseDemoWorkflow(JSON.parse(JSON.stringify(workflow)));

      expect(imported).toEqual(workflow);
      if (promptInactivity === undefined) {
        expect(
          imported.nodes.find((node) => node.data.kind === "agent")!.data
            .agentConfig,
        ).not.toHaveProperty("promptInactivity");
      }
    },
  );

  it.each(["forever", "", true, 0, {}, []])(
    "rejects the invalid policy %j at the workflow import boundary",
    (promptInactivity) => {
      const workflow = createMockWorkflow("en-US");
      const config = workflow.nodes.find((node) => node.data.kind === "agent")!
        .data.agentConfig!;
      Object.assign(config, { promptInactivity });

      expect(() => parseDemoWorkflow(workflow)).toThrow();
    },
  );
});
