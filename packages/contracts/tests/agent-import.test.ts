import assert from "node:assert/strict";
import test from "node:test";

import { createContractsClient } from "../src/client.js";
import type {
  ContractTransport,
  ContractTransportRequest,
} from "../src/transport.js";

test("forwards Agent Markdown and conflict metadata as one IPC request", async () => {
  const requests: ContractTransportRequest[] = [];
  const transport: ContractTransport = {
    async send<TResponse>(
      request: ContractTransportRequest,
    ): Promise<TResponse> {
      requests.push(request);
      return {} as TResponse;
    },
    stream<TEvent>(): AsyncIterable<TEvent> {
      throw new Error("stream was not expected in this test");
    },
  };
  const client = createContractsClient(transport);
  const content =
    "---\nname: review-agent\ndescription: Reviews changes\n---\nReview changes.\n";

  await client.agentImport.prepare({ content });
  await client.agentImport.commit({
    content,
    decision: "overwrite",
    expectedAgentId: "agent-1",
    expectedUpdatedAt: 42,
  });

  assert.deepEqual(requests, [
    {
      operationName: "prepareAgentImport",
      request: { content },
    },
    {
      operationName: "commitAgentImport",
      request: {
        content,
        decision: "overwrite",
        expectedAgentId: "agent-1",
        expectedUpdatedAt: 42,
      },
    },
  ]);
});
