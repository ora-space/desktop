import assert from "node:assert/strict";
import test from "node:test";

import { defineAgentPlugin } from "../../src/agent/index.js";

test("defineAgentPlugin is a structural identity helper", () => {
  const definition = {
    kind: "agent" as const,
    pluginApi: 1 as const,
    activate: async () => ({ providers: [] }),
    authorMetadata: "preserved",
  };

  assert.equal(defineAgentPlugin(definition), definition);
  assert.equal(definition.authorMetadata, "preserved");
});
