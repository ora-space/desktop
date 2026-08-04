import type { WorkspaceFileEventBatch } from "@ora/contracts";
import { describe, expect, it } from "vitest";
import {
  workspaceWatchReconnectDelay,
  watchWorkspaceContinuously,
} from "./workspace-watch";

const BATCH: WorkspaceFileEventBatch = {
  changes: [{ kind: "modified", path: "src/main.rs" }],
};

describe("watchWorkspaceContinuously", () => {
  it("reconnects after failure and resets the retry loop after receiving an event", async () => {
    const controller = new AbortController();
    const waits: number[] = [];
    let connections = 0;
    const openStream = () => {
      connections += 1;
      if (connections === 1) {
        return (async function* () {
          yield* [] as WorkspaceFileEventBatch[];
          throw new Error("disconnected");
        })();
      }
      return (async function* () {
        yield BATCH;
      })();
    };

    await watchWorkspaceContinuously({
      signal: controller.signal,
      openStream,
      onBatch: () => controller.abort(),
      wait: async (delayMs) => {
        waits.push(delayMs);
      },
    });

    expect(connections).toBe(2);
    expect(waits).toEqual([500]);
  });

  it("caps reconnect backoff at ten seconds", () => {
    expect([0, 1, 2, 3, 4, 5, 20].map(workspaceWatchReconnectDelay)).toEqual([
      500,
      1_000,
      2_000,
      4_000,
      8_000,
      10_000,
      10_000,
    ]);
  });
});
