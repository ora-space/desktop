import { HostSimulator } from "../src/testing/mod.ts";
import { AGENT_CONTRACT_VERSION } from "../src/agent/mod.ts";
import { PLUGIN_API_VERSION, SDK_VERSION } from "../src/mod.ts";

function assertEquals(actual: unknown, expected: unknown): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`Expected ${expectedJson}, received ${actualJson}`);
  }
}

Deno.test("HostSimulator drives a real agent plugin process end to end", async () => {
  const host = await HostSimulator.launch({
    entrypoint: new URL("./fixtures/echo-agent.ts", import.meta.url),
    permissions: [],
  });
  assertEquals(host.registration, {
    methods: ["agent/start", "agent/stop", "agent/listModels"],
    emits: ["agent/acp"],
    sdkVersion: SDK_VERSION,
    contracts: { pluginApi: PLUGIN_API_VERSION, agent: AGENT_CONTRACT_VERSION },
  });

  const started = await host.request("agent/start", {
    cwd: Deno.cwd(),
    hostVersion: "0.9.0",
  });
  assertEquals(started.result, { protocol: "acp", acpVersion: 1 });

  const initialized = await host.acpRequest(1, "initialize", {
    protocolVersion: 1,
  });
  assertEquals(initialized, {
    jsonrpc: "2.0",
    id: 1,
    result: { echoed: "initialize", started: 1 },
  });

  const models = await host.request("agent/listModels");
  assertEquals(models.result, {
    models: [{ id: "echo/one", displayName: "one", default: true }],
  });

  const stopped = await host.request("agent/stop");
  assertEquals(stopped.result, {});
  assertEquals(await host.shutdown(), 0);
});
