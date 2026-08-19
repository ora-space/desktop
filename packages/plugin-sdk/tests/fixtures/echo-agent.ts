/**
 * A minimal agent plugin used to exercise the SDK end to end through a real Deno process.
 *
 * It echoes every ACP request back as a response and answers a fixed model list, which is enough
 * to prove the handshake, request/response, notification pass-through, and shutdown paths.
 */
import {
  type AcpSender,
  type AgentModel,
  AgentPlugin,
  type AgentStartContext,
  runAgentPlugin,
} from "../../src/agent/mod.ts";
import type { JsonValue } from "../../src/mod.ts";

class EchoAgent extends AgentPlugin {
  #send: AcpSender | undefined;
  #started = 0;

  override onStart(_context: AgentStartContext, send: AcpSender): void {
    this.#send = send;
    this.#started += 1;
  }

  override onAcp(frame: JsonValue): Promise<void> | void {
    const request = frame as { id?: JsonValue; method?: string };
    if (request.id === undefined) {
      return;
    }
    return this.#send?.({
      jsonrpc: "2.0",
      id: request.id,
      result: { echoed: request.method ?? null, started: this.#started },
    });
  }

  override onListModels(): AgentModel[] {
    return [{ id: "echo/one", displayName: "one", default: true }];
  }
}

await runAgentPlugin(new EchoAgent(), { pluginId: "test.echo" });
