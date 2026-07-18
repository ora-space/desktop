import assert from "node:assert/strict";
import { PassThrough } from "node:stream";
import test from "node:test";

import { BootstrapSession } from "../../src/bootstrap/session.js";
import { encodeJsonFrame, FrameDecoder, type Frame } from "../../src/transport/frame.js";

const encoder = new TextEncoder();

test("business cancellation is single-flight and follows the cancelled turn terminal", async () => {
  let releaseTurn: (() => void) | undefined;
  let cancelCount = 0;
  const provider = createProvider({
    async *sendMessage() {
      await new Promise<void>((resolve) => {
        releaseTurn = resolve;
      });
      return { conversationId: "conversation-1", finishReason: "cancelled" } as const;
    },
    async cancelConversation() {
      cancelCount += 1;
      releaseTurn?.();
      return { disposition: "accepted" } as const;
    },
  });
  const harness = await RuntimeHarness.start(provider);

  harness.send("h:3", "agent.sendMessage", sendMessageParams());
  await waitFor(() => releaseTurn !== undefined);
  harness.send("h:4", "agent.cancelConversation", cancelParams());
  harness.send("h:5", "agent.cancelConversation", cancelParams());

  assert.deepEqual(await harness.output.nextJson(), {
    jsonrpc: "2.0",
    id: "h:3",
    result: { conversationId: "conversation-1", finishReason: "cancelled" },
  });
  const cancellations = [await harness.output.nextJson(), await harness.output.nextJson()];
  assert.deepEqual(
    cancellations.sort((left, right) => responseId(left).localeCompare(responseId(right))),
    [
      { jsonrpc: "2.0", id: "h:4", result: { disposition: "accepted" } },
      { jsonrpc: "2.0", id: "h:5", result: { disposition: "accepted" } },
    ],
  );
  assert.equal(cancelCount, 1);
  await harness.stop("h:6");
});

test("a conversation admits at most one sendMessage turn", async () => {
  let releaseTurn: (() => void) | undefined;
  let sendCount = 0;
  const provider = createProvider({
    async *sendMessage() {
      sendCount += 1;
      await new Promise<void>((resolve) => {
        releaseTurn = resolve;
      });
      return { conversationId: "conversation-1", finishReason: "completed" } as const;
    },
  });
  const harness = await RuntimeHarness.start(provider);

  harness.send("h:3", "agent.sendMessage", sendMessageParams());
  await waitFor(() => releaseTurn !== undefined);
  harness.send("h:4", "agent.sendMessage", sendMessageParams());
  assert.deepEqual(await harness.output.nextJson(), {
    jsonrpc: "2.0",
    id: "h:4",
    error: { code: -32010, message: "Conversation already has an active turn" },
  });
  releaseTurn?.();
  assert.deepEqual(await harness.output.nextJson(), {
    jsonrpc: "2.0",
    id: "h:3",
    result: { conversationId: "conversation-1", finishReason: "completed" },
  });
  assert.equal(sendCount, 1);
  await harness.stop("h:5");
});

test("cancelConversation returns alreadyStopped locally when no active turn exists", async () => {
  let cancelCount = 0;
  const provider = createProvider({
    async cancelConversation() {
      cancelCount += 1;
      return { disposition: "accepted" } as const;
    },
  });
  const harness = await RuntimeHarness.start(provider);

  harness.send("h:3", "agent.cancelConversation", cancelParams());
  assert.deepEqual(await harness.output.nextJson(), {
    jsonrpc: "2.0",
    id: "h:3",
    result: { disposition: "alreadyStopped" },
  });
  assert.equal(cancelCount, 0);
  await harness.stop("h:4");
});

test("invalid nested result data is runtime-fatal", async () => {
  const provider = createProvider({
    async discoverInstallations() {
      return { installations: [], diagnostics: [] };
    },
  });
  const harness = await RuntimeHarness.start(provider);

  harness.send("h:3", "agent.discoverInstallations", {
    providerId: "example",
    scope: { type: "global" },
  });
  await assert.rejects(harness.running, /empty discovery requires a notFound diagnostic/u);
});

test("startConversation must bind its conversation in the first event", async () => {
  const provider = createProvider({
    async *startConversation() {
      yield { kind: "textDelta", channel: "assistant", text: "too early" } as const;
      return { conversationId: "conversation-1", finishReason: "completed" } as const;
    },
  });
  const harness = await RuntimeHarness.start(provider);

  harness.send("h:3", "agent.startConversation", {
    providerId: "example",
    installationId: "installation-1",
    scope: { type: "global" },
    clientRequestId: "00000000-0000-4000-8000-000000000001",
    prompt: "hello",
  });
  await assert.rejects(
    harness.running,
    /startConversation must identify the conversation before other events/u,
  );
});

function createProvider(overrides: Record<string, unknown>): Record<string, unknown> {
  return {
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
      return { conversationId: "conversation-1", finishReason: "completed" } as const;
    },
    async *sendMessage() {
      return { conversationId: "conversation-1", finishReason: "completed" } as const;
    },
    async cancelConversation() {
      return { disposition: "alreadyStopped" } as const;
    },
    ...overrides,
  };
}

function sendMessageParams(): Record<string, unknown> {
  return {
    providerId: "example",
    installationId: "installation-1",
    conversationId: "conversation-1",
    scope: { type: "global" },
    clientRequestId: "00000000-0000-4000-8000-000000000001",
    prompt: "hello",
  };
}

function cancelParams(): Record<string, unknown> {
  return {
    providerId: "example",
    installationId: "installation-1",
    conversationId: "conversation-1",
    scope: { type: "global" },
  };
}

function initializeParams(): Record<string, unknown> {
  return {
    wireVersion: 1,
    hostVersion: "0.1.0",
    runtimeVersion: "1.0.0",
    sessionId: "session-active-turns",
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

class RuntimeHarness {
  readonly output: FrameCollector;
  readonly running: Promise<void>;
  readonly #stdin: PassThrough;

  private constructor(stdin: PassThrough, output: FrameCollector, running: Promise<void>) {
    this.#stdin = stdin;
    this.output = output;
    this.running = running;
  }

  /** Starts and completes the private initialize/activate handshake for one provider fixture. */
  static async start(provider: Record<string, unknown>): Promise<RuntimeHarness> {
    const stdin = new PassThrough();
    const stdout = new PassThrough();
    const stderr = new PassThrough();
    const output = new FrameCollector(stdout);
    const session = new BootstrapSession(
      { stdin, stdout, stderr },
      {
        importer: async () => ({
          default: {
            kind: "agent",
            pluginApi: 1,
            async activate() {
              return { providers: [provider] };
            },
          },
        }),
      },
    );
    const harness = new RuntimeHarness(stdin, output, session.run());
    harness.send("h:1", "$/initialize", initializeParams());
    await output.nextJson();
    harness.send("h:2", "$/activate", { reason: "manualStart" });
    await output.nextJson();
    return harness;
  }

  /** Sends one strict framed Host request. */
  send(id: string, method: string, params: Record<string, unknown>): void {
    const payload = encoder.encode(JSON.stringify({ jsonrpc: "2.0", id, method, params }));
    this.#stdin.write(encodeJsonFrame(payload));
  }

  /** Completes graceful lifecycle shutdown after all asserted business work has settled. */
  async stop(id: string): Promise<void> {
    this.send(id, "$/deactivate", { reason: "manualStop" });
    await this.output.nextJson();
    const payload = encoder.encode(JSON.stringify({ jsonrpc: "2.0", method: "$/exit" }));
    this.#stdin.write(encodeJsonFrame(payload));
    await this.running;
  }
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

  /** Decodes one complete response or notification without treating pipe chunks as messages. */
  async nextJson(): Promise<unknown> {
    const frame =
      this.#frames.shift() ??
      (await Promise.race([
        new Promise<Frame>((resolve) => this.#waiters.push(resolve)),
        new Promise<never>((_resolve, reject) => {
          setTimeout(() => reject(new Error("timed out waiting for bootstrap frame")), 3000);
        }),
      ]));
    return JSON.parse(new TextDecoder().decode(frame.payload)) as unknown;
  }
}

async function waitFor(condition: () => boolean): Promise<void> {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    if (condition()) {
      return;
    }
    await new Promise<void>((resolve) => {
      setTimeout(resolve, 0);
    });
  }
  throw new Error("timed out waiting for active-turn fixture state");
}

function responseId(value: unknown): string {
  if (typeof value !== "object" || value === null || !("id" in value)) {
    throw new TypeError("expected response object with id");
  }
  return String(value.id);
}
