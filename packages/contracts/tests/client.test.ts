import assert from "node:assert/strict";
import test from "node:test";

import { createContractsClient } from "../src/client.ts";
import { endpoints } from "../src/endpoints.ts";
import type {
  ContractCallOptions,
  ContractTransport,
  ContractTransportRequest,
} from "../src/transport.ts";

/** Builds a transport double that records requests and returns a fixed response. */
function recordingTransport<TResponse>(
  requests: ContractTransportRequest[],
  response: TResponse,
): ContractTransport {
  return {
    async send<TTransportResponse>(
      request: ContractTransportRequest,
    ): Promise<TTransportResponse> {
      requests.push(request);
      return response as unknown as TTransportResponse;
    },
    stream<TEvent>(): AsyncIterable<TEvent> {
      throw new Error("stream was not expected in this test");
    },
  };
}

test("forwards the complete operation request to the transport", async () => {
  const requests: ContractTransportRequest[] = [];
  const request = {
    taskId: "task-1",
    title: "Ship SDK",
  };
  const response = {
    task: {
      id: "task-1",
      projectId: "project-1",
      title: "Ship SDK",
    },
  };
  const client = createContractsClient(recordingTransport(requests, response));

  assert.deepEqual(await client.task.update(request), response);
  assert.deepEqual(requests, [
    {
      operationName: "updateTask",
      request,
    },
  ]);
});

test("preserves every field in the complete IPC request", async () => {
  const requests: ContractTransportRequest[] = [];
  const request = {
    sessionId: "session-1",
    decisions: [{ candidateId: "candidate-1", decision: "skip" as const }],
  };
  const client = createContractsClient(
    recordingTransport(requests, {
      sessionId: "session-1",
      status: "committing" as const,
      progress: { processed: 0, total: 1, results: [] },
    }),
  );

  await client.skillImport.commit(request);

  assert.deepEqual(requests, [{ operationName: "commitSkillImport", request }]);
});

test("forwards call options through unary operations", async () => {
  const controller = new AbortController();
  let observedSignal: AbortSignal | undefined;
  const transport: ContractTransport = {
    async send<TResponse>(
      _request: ContractTransportRequest,
      options?: ContractCallOptions,
    ): Promise<TResponse> {
      observedSignal = options?.signal;
      return { projects: [] } as TResponse;
    },
    stream<TEvent>(): AsyncIterable<TEvent> {
      throw new Error("stream was not expected in this test");
    },
  };

  await createContractsClient(transport).project.list(
    {},
    { signal: controller.signal },
  );

  assert.equal(observedSignal, controller.signal);
});

test("passes operation and request to stream transports", async () => {
  let observedRequest: ContractTransportRequest | undefined;
  let observedOptions: ContractCallOptions | undefined;
  const options = { signal: new AbortController().signal };
  const transport: ContractTransport = {
    async send<TResponse>(): Promise<TResponse> {
      throw new Error("unary transport was not expected in this test");
    },
    stream<TEvent>(request: ContractTransportRequest, callOptions?: ContractCallOptions): AsyncIterable<TEvent> {
      observedRequest = request;
      observedOptions = callOptions;
      return (async function* () {
        yield { type: "Ready" } as TEvent;
      })();
    },
  };
  const request = { sessionId: "session-1" };

  const events = createContractsClient(transport).session.load(request, options);
  const received: unknown[] = [];
  for await (const event of events) received.push(event);
  assert.deepEqual(received, [{ type: "Ready" }]);
  assert.deepEqual(observedRequest, {
    operationName: "loadSession",
    request,
  });
  assert.equal(observedOptions, options);
});

test("routes every generated member through its declared mode with the complete DTO and options", async () => {
  const calls: unknown[] = [];
  const options = { signal: new AbortController().signal };
  const response = { fixture: "response" };
  const transport: ContractTransport = {
    async send<T>(request: ContractTransportRequest, callOptions?: ContractCallOptions): Promise<T> {
      calls.push({ mode: "unary", request, options: callOptions });
      return response as T;
    },
    async *stream<T>(request: ContractTransportRequest, callOptions?: ContractCallOptions): AsyncIterable<T> {
      calls.push({ mode: "stream", request, options: callOptions });
      yield response as T;
    },
  };
  // Deliberately enumerate dynamically: static factory typing separately verifies every DTO shape.
  const client = createContractsClient(transport) as unknown as Record<string, Record<string,
    (request: unknown, options: ContractCallOptions) => Promise<unknown> | AsyncIterable<unknown>
  >>;
  const expected: unknown[] = [];
  for (const endpoint of Object.values(endpoints)) {
    const request = { fixture: endpoint.operationName, nested: { preserved: true } };
    const result = client[endpoint.namespace][endpoint.memberName](request, options);
    if (endpoint.responseMode === "stream") {
      const events: unknown[] = [];
      for await (const event of result as AsyncIterable<unknown>) events.push(event);
      assert.deepEqual(events, [response]);
    } else {
      assert.deepEqual(await result, response);
    }
    expected.push({ mode: endpoint.responseMode, request: { operationName: endpoint.operationName, request }, options });
  }
  assert.deepEqual(calls, expected);
});

test("exposes every generated endpoint in its declared namespace", () => {
  const client = createContractsClient(recordingTransport([], {}));
  const clientRecord = client as unknown as Record<
    string,
    Record<string, unknown>
  >;

  for (const endpoint of Object.values(endpoints)) {
    assert.equal(
      typeof clientRecord[endpoint.namespace]?.[endpoint.memberName],
      "function",
    );
  }
});

test("omits standalone worktree operations from generated contracts", () => {
  assert.equal("createWorktree" in endpoints, false);
  assert.equal("getWorktree" in endpoints, false);
  assert.equal("listWorktrees" in endpoints, false);
  assert.equal("updateWorktree" in endpoints, false);
  assert.equal("deleteWorktree" in endpoints, false);
});
