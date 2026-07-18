import assert from "node:assert/strict";
import test from "node:test";

import { validateInitializeParams } from "../../src/bootstrap/contracts.js";

test("initialize validation aligns path containment and limits with the Rust Host", () => {
  const forwardSlashes = initializeParams();
  assert.deepEqual(validateInitializeParams(forwardSlashes).paths, forwardSlashes.paths);

  const escaping = initializeParams();
  escaping.paths.entryPath = "D:/ora/plugins/ora.example/../other/index.js";
  assert.throws(() => validateInitializeParams(escaping), /strict descendant/u);

  const outside = initializeParams();
  outside.paths.entryPath = "D:/ora/plugins/other/index.js";
  assert.throws(() => validateInitializeParams(outside), /strict descendant/u);

  const invalidLimits = initializeParams();
  invalidLimits.limits.maxActiveTurns = 0;
  assert.throws(() => validateInitializeParams(invalidLimits), /maxActiveTurns/u);
});

interface MutableInitializeFixture {
  wireVersion: number;
  hostVersion: string;
  runtimeVersion: string;
  sessionId: string;
  plugin: Record<string, unknown>;
  paths: { extensionPath: string; entryPath: string; storagePath: string };
  declaredAgents: Array<Record<string, unknown>>;
  limits: Record<string, number>;
}

/** Builds a mutable fixture so each rejection case changes exactly one cross-field invariant. */
function initializeParams(): MutableInitializeFixture {
  return {
    wireVersion: 1,
    hostVersion: "0.1.0",
    runtimeVersion: "1.0.0",
    sessionId: "session-1",
    plugin: {
      id: "ora.example",
      version: "0.1.0",
      kind: "agent",
      pluginApi: 1,
      contentOwner: `sha256-${"a".repeat(64)}`,
    },
    paths: {
      extensionPath: "D:/ora/plugins/ora.example",
      entryPath: "D:/ora/plugins/ora.example/dist/index.js",
      storagePath: "D:/ora/plugin-data/ora.example/owner",
    },
    declaredAgents: [{ id: "example", contractVersion: 1 }],
    limits: {
      maxFrameBytes: 8 * 1024 * 1024,
      maxPendingRequests: 8,
      maxAgentEventBytes: 256 * 1024,
      maxAgentResultBytes: 1024 * 1024,
      maxAgentPromptBytes: 1024 * 1024,
      maxActiveTurns: 4,
      maxPageItems: 100,
    },
  };
}
