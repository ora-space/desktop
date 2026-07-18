import assert from "node:assert/strict";
import test from "node:test";

import type {
  AgentEvent,
  AgentScope,
  DiscoverInstallationsRequest,
} from "../../src/types/index.js";

test("generated Agent DTOs expose the frozen discriminants", () => {
  const scope: AgentScope = { type: "global" };
  const request: DiscoverInstallationsRequest = {
    providerId: "claude-code",
    scope,
  };
  const event: AgentEvent = {
    kind: "textDelta",
    channel: "assistant",
    text: "hello",
  };

  assert.deepEqual(request, { providerId: "claude-code", scope: { type: "global" } });
  assert.deepEqual(event, { kind: "textDelta", channel: "assistant", text: "hello" });
});
