import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import test from "node:test";

import { BootstrapSession } from "../../src/bootstrap/session.js";
import { encodeJsonFrame, FrameDecoder, type Frame } from "../../src/transport/frame.js";

const encoder = new TextEncoder();

test("private bootstrap performs initialize, activate, typed invoke, stream, deactivate, and exit", async () => {
  const stdin = new PassThrough();
  const stdout = new PassThrough();
  const stderr = new PassThrough();
  const output = new FrameCollector(stdout);
  let importCount = 0;
  let deactivateCount = 0;

  const provider = {
    id: "example",
    contractVersion: 1,
    async discoverInstallations() {
      return {
        installations: [],
        diagnostics: [{ kind: "notFound", message: "No installations found" }],
      };
    },
    async getConfigurationSummary() {
      return { items: [] };
    },
    async listSkills() {
      return { items: [] };
    },
    async listMcpServers() {
      return { items: [] };
    },
    async listConversations() {
      return { items: [] };
    },
    async *startConversation() {
      yield { kind: "conversationStarted", conversationId: "conversation-1" } as const;
      yield { kind: "textDelta", channel: "assistant", text: "hello" } as const;
      return { conversationId: "conversation-1", finishReason: "completed" } as const;
    },
    async *sendMessage() {
      yield { kind: "textDelta", channel: "assistant", text: "reply" } as const;
      return { conversationId: "conversation-1", finishReason: "completed" } as const;
    },
    async cancelConversation() {
      return { disposition: "alreadyStopped" } as const;
    },
  };
  const definition = {
    kind: "agent",
    pluginApi: 1,
    async activate() {
      return { providers: [provider] };
    },
    async deactivate() {
      deactivateCount += 1;
    },
  } as const;
  const session = new BootstrapSession(
    { stdin, stdout, stderr },
    {
      importer: async () => {
        importCount += 1;
        return { default: definition };
      },
    },
  );
  const running = session.run();

  sendRequest(stdin, "h:1", "$/initialize", initializeParams());
  assert.deepEqual(await output.nextJson(), {
    jsonrpc: "2.0",
    id: "h:1",
    result: {
      wireVersion: 1,
      runtimeVersion: "1.0.0",
      sessionId: "session-1",
      plugin: { id: "ora.example", version: "0.1.0" },
    },
  });
  assert.equal(importCount, 0);

  sendRequest(stdin, "h:2", "$/activate", { reason: "manualStart" });
  assert.deepEqual(await output.nextJson(), {
    jsonrpc: "2.0",
    id: "h:2",
    result: { providers: [{ id: "example", contractVersion: 1 }] },
  });
  assert.equal(importCount, 1);

  sendRequest(stdin, "h:3", "agent.discoverInstallations", {
    providerId: "example",
    scope: { type: "global" },
  });
  assert.deepEqual(await output.nextJson(), {
    jsonrpc: "2.0",
    id: "h:3",
    result: {
      installations: [],
      diagnostics: [{ kind: "notFound", message: "No installations found" }],
    },
  });

  sendRequest(stdin, "h:4", "agent.startConversation", {
    providerId: "example",
    installationId: "installation-1",
    scope: { type: "global" },
    clientRequestId: "00000000-0000-4000-8000-000000000001",
    prompt: "hello",
  });
  assert.deepEqual(await output.nextJson(), {
    jsonrpc: "2.0",
    method: "$/stream",
    params: {
      id: "h:4",
      seq: 1,
      value: { kind: "conversationStarted", conversationId: "conversation-1" },
    },
  });
  assert.deepEqual(await output.nextJson(), {
    jsonrpc: "2.0",
    method: "$/stream",
    params: {
      id: "h:4",
      seq: 2,
      value: { kind: "textDelta", channel: "assistant", text: "hello" },
    },
  });
  assert.deepEqual(await output.nextJson(), {
    jsonrpc: "2.0",
    id: "h:4",
    result: { conversationId: "conversation-1", finishReason: "completed" },
  });

  sendRequest(stdin, "h:5", "$/deactivate", { reason: "manualStop" });
  assert.deepEqual(await output.nextJson(), { jsonrpc: "2.0", id: "h:5", result: {} });
  sendNotification(stdin, "$/exit");
  await running;
  assert.equal(deactivateCount, 1);
});

function initializeParams(): Record<string, unknown> {
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
      extensionPath: "D:\\ora\\plugins\\ora.example",
      entryPath: "D:\\ora\\plugins\\ora.example\\dist\\index.js",
      storagePath: "D:\\ora\\plugin-data\\ora.example\\owner",
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

function sendRequest(
  stdin: PassThrough,
  id: string,
  method: string,
  params: Record<string, unknown>,
): void {
  const payload = encoder.encode(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
  stdin.write(encodeJsonFrame(payload));
}

function sendNotification(stdin: PassThrough, method: string): void {
  const payload = encoder.encode(JSON.stringify({ jsonrpc: "2.0", method }));
  stdin.write(encodeJsonFrame(payload));
}

class FrameCollector {
  readonly #decoder = new FrameDecoder();
  readonly #frames: Frame[] = [];
  readonly #waiters: Array<(frame: Frame) => void> = [];

  constructor(stream: PassThrough) {
    stream.on("data", (chunk: Buffer) => {
      for (const frame of this.#decoder.decodeChunk(new Uint8Array(chunk))) {
        const waiter = this.#waiters.shift();
        if (waiter === undefined) {
          this.#frames.push(frame);
        } else {
          waiter(frame);
        }
      }
    });
  }

  async nextJson(): Promise<unknown> {
    const frame = this.#frames.shift() ?? (await new Promise<Frame>((resolve) => this.#waiters.push(resolve)));
    return JSON.parse(new TextDecoder().decode(frame.payload)) as unknown;
  }
}
