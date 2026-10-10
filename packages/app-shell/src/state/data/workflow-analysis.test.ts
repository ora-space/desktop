import { act, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";
import type { AnalyzeWorkflowResponse } from "@ora/contracts";
import "../../i18n/i18n-instance";
import { createTestClient } from "../../test/contracts-transport";
import { renderHookWithClient } from "../../test/hook-harness";
import {
  excludedWorkflowNodeIds,
  unrecognizedWorkflowKinds,
  useWorkflowAnalysis,
} from "./workflow-analysis";

describe("workflow execution membership", () => {
  it("badges spare nodes and unrecognized kinds without listing an id twice", () => {
    const analysis = {
      unusedNodeIds: ["spare", "shared"],
      unrecognizedNodes: [
        { nodeId: "shared", kind: "hologram" },
        { nodeId: "ghost", kind: "hologram" },
        { nodeId: "other", kind: "portal" },
      ],
    };
    expect(excludedWorkflowNodeIds(analysis)).toEqual([
      "spare",
      "shared",
      "ghost",
      "other",
    ]);
    expect(unrecognizedWorkflowKinds(analysis)).toBe("hologram, portal");
  });

  it("treats a missing analysis as nothing excluded", () => {
    expect(excludedWorkflowNodeIds(undefined)).toEqual([]);
    expect(unrecognizedWorkflowKinds(undefined)).toBe("");
  });
});

describe("workflow execution analysis", () => {
  it("cancels superseded documents and ignores their late responses", async () => {
    const requests: Array<{
      graph: string;
      signal: AbortSignal | undefined;
      resolve: (value: AnalyzeWorkflowResponse) => void;
    }> = [];
    const client = createTestClient({
      analyzeWorkflow: ({ graph }, options) =>
        new Promise((resolve) => {
          requests.push({ graph, signal: options?.signal, resolve });
        }),
    });
    let document = "first";
    let owner = "workflow-a";
    const hook = renderHookWithClient(
      () => useWorkflowAnalysis(owner, document),
      client,
    );
    await waitFor(() => expect(requests).toHaveLength(1));
    owner = "workflow-b";
    document = "second";
    hook.rerender();
    await waitFor(() => expect(requests).toHaveLength(2));
    expect(requests[0].signal?.aborted).toBe(true);
    expect(hook.result.current.data).toBeUndefined();
    await act(async () =>
      requests[1].resolve({
        unusedNodeIds: ["current"],
        unrecognizedNodes: [],
      }),
    );
    await waitFor(() =>
      expect(hook.result.current.data).toEqual({
        unusedNodeIds: ["current"],
        unrecognizedNodes: [],
      }),
    );
    await act(async () =>
      requests[0].resolve({ unusedNodeIds: ["stale"], unrecognizedNodes: [] }),
    );
    expect(hook.result.current.data).toEqual({
      unusedNodeIds: ["current"],
      unrecognizedNodes: [],
    });
    hook.unmount();
  });

  it("aborts analysis when its owner unmounts", async () => {
    let signal: AbortSignal | undefined;
    const client = createTestClient({
      analyzeWorkflow: (_request, options) => {
        signal = options?.signal;
        return new Promise(() => {});
      },
    });
    const hook = renderHookWithClient(
      () => useWorkflowAnalysis("workflow", "graph"),
      client,
    );
    await waitFor(() => expect(signal).toBeDefined());
    hook.unmount();
    expect(signal?.aborted).toBe(true);
  });

  it("exposes structural failures without inventing an unused-node result", async () => {
    const client = createTestClient({
      analyzeWorkflow: () => {
        throw new Error("dangling edge");
      },
    });
    const hook = renderHookWithClient(
      () => useWorkflowAnalysis("workflow", "invalid"),
      client,
    );
    await waitFor(() => expect(hook.result.current.isError).toBe(true));
    expect(hook.result.current.data).toBeUndefined();
    hook.unmount();
  });
});
