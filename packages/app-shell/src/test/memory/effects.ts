import type { TestHandlers } from "../contracts-transport";

/** Registers only the effects operations explicitly requested by a fixture. */
export function readyEffectHandlers() {
  return {
    getEffectTargetStatus: async () => ({
      status: {
        targetId: "mock-effect-target",
        desiredGeneration: 1,
        observedGeneration: 1,
        appliedGeneration: 1,
        readyGeneration: 1,
        phase: "current",
        statusVersion: 1,
        recoveryOperationId: null,
        updatedAt: 1n,
        conditions: [],
      },
    }),
  } satisfies TestHandlers;
}
