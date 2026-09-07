import type {
  GetWorkflowSnapshotResponse,
  PromptSessionEvent,
  PromptSessionRequest,
} from "@ora/contracts";
import { describe, expect, it, vi } from "vitest";
import {
  createTestClient,
  createTestTransport,
  type TestHandlers,
  type OperationHandler,
} from "./contracts-transport";

describe("typed test transport", () => {
  it("uses generated namespace wiring and forwards the original DTO, response, and options", async () => {
    const controller = new AbortController();
    const request = { snapshotId: "snapshot-1" };
    const options = { signal: controller.signal };
    const response: GetWorkflowSnapshotResponse = {
      snapshot: {
        id: "snapshot-1",
        workflowId: "workflow-1",
        version: "v1",
        graph: "{}",
        createdAt: 9_007_199_254_740_993n,
        updatedAt: null,
      },
    };
    const getSnapshot = vi.fn<OperationHandler<"getWorkflowSnapshot">>(
      () => response,
    );
    const client = createTestClient({ getWorkflowSnapshot: getSnapshot });
    expect(await client.workflow.getSnapshot(request, options)).toBe(response);
    expect(getSnapshot.mock.calls[0]).toEqual([request, options]);
    expect(getSnapshot.mock.calls[0]?.[0]).toBe(request);
    expect(getSnapshot.mock.calls[0]?.[1]).toBe(options);
  });

  it("does not substitute empty success for an omitted unary operation", async () => {
    const client = createTestClient({ listProjects: () => ({ projects: [] }) });
    expect(await client.project.list({})).toEqual({ projects: [] });
    await expect(client.session.list({})).rejects.toThrow(
      "Unconfigured test operation: listSessions",
    );
  });

  it("fails an omitted stream only when consumption begins", async () => {
    const client = createTestClient({});
    const stream = client.appEvents.watch({});
    await expect(stream[Symbol.asyncIterator]().next()).rejects.toThrow(
      "Unconfigured test operation: watchAppEvents",
    );
  });

  it("starts streams lazily and finalizes the handler when the consumer exits", async () => {
    const events: PromptSessionEvent[] = [
      { type: "completed", stopReason: "end_turn" },
    ];
    const request: PromptSessionRequest = {
      sessionId: "session-1",
      prompt: [],
    };
    const options = { signal: new AbortController().signal };
    const received: unknown[] = [];
    let finalized = false;
    const client = createTestClient({
      promptSession: async function* (dto, callOptions) {
        received.push(dto, callOptions);
        try {
          yield* events;
        } finally {
          finalized = true;
        }
      },
    });
    const stream = client.session.prompt(request, options);
    expect(received).toEqual([]);
    for await (const event of stream) {
      expect(event).toBe(events[0]);
      break;
    }
    expect(received).toEqual([request, options]);
    expect(finalized).toBe(true);
  });

  it("rejects unknown operations, wrong modes, and inherited handlers", async () => {
    const transport = createTestTransport(
      Object.create({ listProjects: () => ({ projects: [] }) }),
    );
    await expect(
      transport.send({ operationName: "listProjects", request: {} }),
    ).rejects.toThrow("Unconfigured test operation");
    await expect(
      transport.send({ operationName: "toString", request: {} }),
    ).rejects.toThrow("Unknown test operation");
    await expect(
      transport.send({ operationName: "watchAppEvents", request: {} }),
    ).rejects.toThrow("Wrong test transport mode");
  });

  it("checks each operation's request and response mode at registration", () => {
    const invalid: TestHandlers = {
      // @ts-expect-error Project list handlers cannot receive task deletion requests.
      listProjects: (request: { taskId: string }) => {
        void request;
        return { projects: [] };
      },
      // @ts-expect-error A unary session list cannot return a stream.
      listSessions: async function* () {
        yield { sessions: [] };
      },
      // @ts-expect-error A stream handler must return an AsyncIterable of typed events.
      watchAppEvents: async () => ({ type: "ready" }),
    };
    expect(Object.keys(invalid)).toEqual([
      "listProjects",
      "listSessions",
      "watchAppEvents",
    ]);
  });
});
