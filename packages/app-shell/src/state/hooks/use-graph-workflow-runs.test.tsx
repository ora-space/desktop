import { act, waitFor } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import { createChatStore } from "@ora/chat";
import { createMockWorkflow } from "@ora/workflow-mock";
import { normalizeWorkflowDefinition } from "@ora/workflow-runtime";
import { createMemoryWorkflowRuntime } from "@ora/workflow-runtime/memory";
import { createMockClient, createMockClientState } from "../../test/mock-client";
import {
  createTestQueryClient,
  renderHookWithClient,
} from "../../test/hook-harness";
import { useGraphWorkflowRunLive } from "./use-graph-workflow-runs";

describe("useGraphWorkflowRunLive", () => {
  it("observes finish events after loading the matching cursor snapshot", async () => {
    const runtime = createMemoryWorkflowRuntime({ autoStart: false });
    const definition = normalizeWorkflowDefinition(createMockWorkflow("zh-CN"));
    await runtime.host.mount("p1", definition);
    const run = await runtime.runs.create({
      projectId: "p1",
      definitionId: definition.id,
    });
    const onRunFinished = vi.fn();
    const client = createMockClient(createMockClientState());
    const queryClient = createTestQueryClient();
    const chatStore = createChatStore(client.session);
    const { result, unmount } = renderHookWithClient(
      () => useGraphWorkflowRunLive(run.id, { onRunFinished }),
      client,
      queryClient,
      chatStore,
      runtime,
    );
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    const initialCursor = result.current.data?.cursor;

    await act(async () => runtime.runs.cancel(run.id));

    expect(onRunFinished).toHaveBeenCalledOnce();
    await waitFor(() => expect(result.current.data?.cursor).not.toBe(initialCursor));
    unmount();

    const onRemountedRunFinished = vi.fn();
    const remounted = renderHookWithClient(
      () => useGraphWorkflowRunLive(run.id, {
        onRunFinished: onRemountedRunFinished,
      }),
      client,
      queryClient,
      chatStore,
      runtime,
    );
    await waitFor(() => expect(remounted.result.current.isSuccess).toBe(true));
    expect(onRemountedRunFinished).not.toHaveBeenCalled();
    remounted.unmount();
    runtime.dispose();
  });
});
