import { renderHook, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createChatStore } from "@ora/chat";
import { createMockWorkflow } from "@ora/workflow-mock";
import {
  normalizeWorkflowDefinition,
  type GraphWorkflowRun,
} from "@ora/workflow-runtime";
import { createMemoryWorkflowRuntime } from "@ora/workflow-runtime/memory";
import { createMockClient, createMockClientState } from "../../test/mock-client";
import {
  createHookWrapper,
  createTestQueryClient,
} from "../../test/hook-harness";
import { useTheaterHitl } from "./use-theater-hitl";

/** Builds one waiting run so controller lifecycle can be tested without engine timers. */
function waitingRun(id: string, requestId: string): GraphWorkflowRun {
  const definition = normalizeWorkflowDefinition(createMockWorkflow("en-US"));
  return {
    id,
    projectId: "p1",
    definitionId: definition.id,
    definitionSnapshot: definition,
    name: definition.name,
    status: "awaiting_input",
    nodeStates: Object.fromEntries(
      definition.nodes.map((node) => [
        node.id,
        { status: node.id === "understand" ? "awaiting_input" as const : "idle" as const },
      ]),
    ),
    openHitls: [{
      id: requestId,
      runId: id,
      nodeId: "understand",
      schema: {
        kind: "clarify",
        title: "Clarify",
        fields: [{ name: "answer", type: "text", label: "Answer", required: true }],
      },
      blocking: true,
      policy: "wait",
      status: "open",
      createdAt: "2026-08-04T12:00:00+08:00",
    }],
    totals: {},
    createdAt: "2026-08-04T12:00:00+08:00",
    updatedAt: "2026-08-04T12:00:00+08:00",
  };
}

describe("useTheaterHitl", () => {
  it("resets drafts then re-engages HITL when switching to another waiting run", async () => {
    const client = createMockClient(createMockClientState());
    const runtime = createMemoryWorkflowRuntime();
    const wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
      runtime,
    );
    const first = waitingRun("run-1", "hitl-1");
    const second = waitingRun("run-2", "hitl-2");
    const onFocusNode = vi.fn();
    const initialProps: { run: GraphWorkflowRun; focusNodeId: string | null } = {
      run: first,
      focusNodeId: "understand",
    };
    const { result, rerender, unmount } = renderHook(
      ({ run, focusNodeId }: { run: GraphWorkflowRun; focusNodeId: string | null }) =>
        useTheaterHitl({
          run,
          focusNodeId,
          primaryId: "understand",
          onFocusNode,
        }),
      {
        initialProps,
        wrapper,
      },
    );
    await waitFor(() => expect(result.current.hitlExpanded).toBe(true));

    rerender({ run: second, focusNodeId: null });

    // Cold-open / run switch onto an already-waiting run expands the gate again.
    await waitFor(() => {
      expect(result.current.hitlExpanded).toBe(true);
      expect(result.current.openHitls[0]?.id).toBe("hitl-2");
    });
    unmount();
    runtime.dispose();
  });

  it("collapses HITL when browsing away from the waiting act", async () => {
    const client = createMockClient(createMockClientState());
    const runtime = createMemoryWorkflowRuntime();
    const wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
      runtime,
    );
    const run = waitingRun("run-1", "hitl-1");
    const onFocusNode = vi.fn();
    const { result, rerender, unmount } = renderHook(
      ({
        focusNodeId,
        primaryId,
      }: {
        focusNodeId: string | null;
        primaryId: string | null;
      }) =>
        useTheaterHitl({
          run,
          focusNodeId,
          primaryId,
          onFocusNode,
        }),
      {
        initialProps: { focusNodeId: "understand", primaryId: "understand" },
        wrapper,
      },
    );
    await waitFor(() => expect(result.current.hitlExpanded).toBe(true));

    rerender({ focusNodeId: "start", primaryId: "start" });

    await waitFor(() => expect(result.current.hitlExpanded).toBe(false));
    unmount();
    runtime.dispose();
  });

  it("keeps HITL collapsed on first discovery when the stage is on another act", async () => {
    const client = createMockClient(createMockClientState());
    const runtime = createMemoryWorkflowRuntime();
    const wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
      runtime,
    );
    const run = waitingRun("run-1", "hitl-1");
    const onFocusNode = vi.fn();
    const { result, unmount } = renderHook(
      () =>
        useTheaterHitl({
          run,
          focusNodeId: "start",
          primaryId: "start",
          onFocusNode,
        }),
      { wrapper },
    );

    await waitFor(() => {
      expect(result.current.openHitls).toHaveLength(1);
      expect(result.current.hitlExpanded).toBe(false);
      expect(result.current.primaryHasHitl).toBe(false);
    });
    unmount();
    runtime.dispose();
  });

  it("expands HITL on first discovery when the stage is already on the waiting act", async () => {
    const client = createMockClient(createMockClientState());
    const runtime = createMemoryWorkflowRuntime();
    const wrapper = createHookWrapper(
      client,
      createTestQueryClient(),
      createChatStore(client.session),
      runtime,
    );
    const run = waitingRun("run-1", "hitl-1");
    const onFocusNode = vi.fn();
    const { result, unmount } = renderHook(
      () =>
        useTheaterHitl({
          run,
          focusNodeId: null,
          primaryId: "understand",
          onFocusNode,
        }),
      { wrapper },
    );

    await waitFor(() => {
      expect(result.current.hitlExpanded).toBe(true);
      expect(result.current.primaryHasHitl).toBe(true);
    });
    unmount();
    runtime.dispose();
  });
});
