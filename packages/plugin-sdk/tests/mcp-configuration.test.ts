import {
  AGENT_CONFIGURE_WORKSPACE,
  defineAgent,
  type McpConfigurationSnapshotRequest,
  parseMcpConfigurationSnapshotRequest,
} from "../src/mod.ts";
import {
  negotiateMcpConfiguration,
  parseMcpConfigurationReceipt,
  parseMcpConfigurationRegistration,
  validateMcpConfigurationReceiptCoverage,
} from "../src/mcp.ts";
import {
  decodeFrames,
  encodeFrame,
  type JsonValue,
  type PluginTransport,
} from "../src/protocol.ts";

/** Compares JSON-compatible values without a Node compatibility dependency. */
function assertEquals(actual: unknown, expected: unknown): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`Expected ${expectedJson}, received ${actualJson}`);
  }
}

function assertStringDoesNotContain(value: string, forbidden: string): void {
  if (value.includes(forbidden)) {
    throw new Error(`Expected diagnostics not to contain ${forbidden}`);
  }
}

/** Creates paired in-memory streams for exercising the SDK without global stdio. */
function createTransportHarness(): {
  transport: PluginTransport;
  send: (message: JsonValue) => Promise<void>;
  responses: AsyncGenerator<unknown>;
} {
  const hostInput = new TransformStream<Uint8Array>();
  const pluginOutput = new TransformStream<Uint8Array>(
    undefined,
    undefined,
    new CountQueuingStrategy({ highWaterMark: Infinity }),
  );
  const inputWriter = hostInput.writable.getWriter();
  return {
    transport: {
      readable: hostInput.readable,
      writable: pluginOutput.writable,
      redirectConsole: false,
    },
    send: (message) => inputWriter.write(encodeFrame(message)),
    responses: decodeFrames(pluginOutput.readable),
  };
}

async function loadFixture(relativePath: string): Promise<JsonValue> {
  const url = new URL(
    `./fixtures/mcp-configuration/${relativePath}`,
    import.meta.url,
  );
  return JSON.parse(await Deno.readTextFile(url));
}

function minimalAgent() {
  return {
    start: () => {},
    stop: () => {},
    listModels: () => [],
    onAcp: () => {},
  };
}

Deno.test("older agents omit mcpConfiguration and configureWorkspace", async () => {
  const plugin = defineAgent(minimalAgent());
  const harness = createTransportHarness();
  const run = plugin.run(harness.transport);
  const registration = (await harness.responses.next()).value as {
    params: JsonValue;
  };
  assertEquals(
    registration.params,
    await loadFixture("registration/omitted.json"),
  );
  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});

Deno.test("defineAgent registers mcpConfiguration together with the snapshot handler", async () => {
  const snapshots: McpConfigurationSnapshotRequest[] = [];
  const plugin = defineAgent({
    ...minimalAgent(),
    mcpConfiguration: {
      protocolVersion: 1,
      transports: ["http"],
      coordination: "wait_for_idle_and_restart",
      configureWorkspace: (request) => {
        snapshots.push(request);
        return {
          appliedGeneration: request.generation,
          documentLocator: ".opencode/opencode.json",
          documentFingerprint:
            "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
          entries: request.resolvedMcps.map((mcp) => ({
            managedIdentity: mcp.managedIdentity,
            nativeKey: "ora_tavily_search_abcdef123456",
            entryFingerprint:
              "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            sourceRevisionId: mcp.sourceRevisionId,
          })),
        };
      },
    },
  });
  const harness = createTransportHarness();
  const run = plugin.run(harness.transport);
  const registration = (await harness.responses.next()).value as {
    params: JsonValue;
  };
  assertEquals(
    registration.params,
    await loadFixture("registration/valid-http-v1.json"),
  );

  const snapshot = await loadFixture("requests/full-snapshot.json");
  await harness.send({
    jsonrpc: "2.0",
    id: 8,
    method: AGENT_CONFIGURE_WORKSPACE,
    params: snapshot,
  });
  assertEquals((await harness.responses.next()).value, {
    jsonrpc: "2.0",
    id: 8,
    result: await loadFixture("receipts/valid.json"),
  });
  assertEquals(snapshots, [snapshot]);
  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});

Deno.test("unknown snapshot protocol versions are rejected without leaking secrets", async () => {
  const plugin = defineAgent({
    ...minimalAgent(),
    mcpConfiguration: {
      protocolVersion: 1,
      transports: ["http"],
      coordination: "wait_for_idle_and_restart",
      configureWorkspace: () => {
        throw new Error("handler must not run");
      },
    },
  });
  const harness = createTransportHarness();
  const run = plugin.run(harness.transport);
  await harness.responses.next();
  const snapshot = await loadFixture("requests/full-snapshot.json") as Record<
    string,
    JsonValue
  >;
  await harness.send({
    jsonrpc: "2.0",
    id: 9,
    method: AGENT_CONFIGURE_WORKSPACE,
    params: { ...snapshot, protocolVersion: 2 },
  });
  const response = (await harness.responses.next()).value as {
    error: { message: string };
  };
  assertEquals(
    response.error.message,
    "agent/configureWorkspace requires a protocol v1 snapshot",
  );
  assertStringDoesNotContain(JSON.stringify(response), "tavily-secret-key");
  assertStringDoesNotContain(JSON.stringify(response), "Bearer");
  await harness.send({ jsonrpc: "2.0", method: "ora/shutdown" });
  await run;
});

Deno.test("snapshot parse rejects unknown fields without echoing header values", () => {
  const snapshot = {
    protocolVersion: 1,
    operationId: "op-7",
    agentTargetId: "target-1",
    workspaceRoot: "/workspace",
    generation: 4,
    resolvedMcps: [],
    configurationStore: { apiKey: "tavily-secret-key" },
  };
  try {
    parseMcpConfigurationSnapshotRequest(snapshot);
    throw new Error("expected parse to fail");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    assertEquals(
      message,
      "agent/configureWorkspace requires a protocol v1 snapshot",
    );
    assertStringDoesNotContain(message, "tavily-secret-key");
    assertStringDoesNotContain(message, "configurationStore");
  }
});

Deno.test("snapshot parse rejects relative HTTP URLs and empty header names", () => {
  const snapshot = {
    protocolVersion: 1,
    operationId: "op-7",
    agentTargetId: "target-1",
    workspaceRoot: "/workspace",
    generation: 4,
    resolvedMcps: [{
      canonicalIdentity: "official/ora-space.tavily-search",
      managedIdentity: "mcp-tavily",
      packageVersion: "0.1.0",
      sourceRevisionId: "rev-tavily-1",
      transport: {
        kind: "http" as const,
        url: "/relative",
        headers: { Authorization: "Bearer tavily-secret-key" } as Record<
          string,
          string
        >,
      },
    }],
  };
  try {
    parseMcpConfigurationSnapshotRequest(snapshot);
    throw new Error("expected relative URL to fail");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    assertEquals(
      message,
      "agent/configureWorkspace requires a protocol v1 snapshot",
    );
    assertStringDoesNotContain(message, "tavily-secret-key");
  }

  snapshot.resolvedMcps[0].transport.url = "https://mcp.tavily.com/mcp";
  snapshot.resolvedMcps[0].transport.headers = {
    "": "Bearer tavily-secret-key",
  };
  try {
    parseMcpConfigurationSnapshotRequest(snapshot);
    throw new Error("expected empty header name to fail");
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    assertEquals(
      message,
      "agent/configureWorkspace requires a protocol v1 snapshot",
    );
    assertStringDoesNotContain(message, "tavily-secret-key");
  }
});

const HTTP_V1 = {
  protocolVersion: 1,
  transports: ["http"],
  coordination: "wait_for_idle_and_restart",
} as const;

Deno.test("shared registration fixtures classify omitted, unknown, malformed, and unpaired declarations", async () => {
  assertEquals(
    parseMcpConfigurationRegistration(
      await loadFixture("registration/omitted.json"),
    ),
    { status: "absent" },
  );
  assertEquals(
    parseMcpConfigurationRegistration(
      await loadFixture("registration/unknown-top-level-fields.json"),
    ),
    { status: "declared", capability: HTTP_V1 },
  );
  assertEquals(
    parseMcpConfigurationRegistration(
      await loadFixture("registration/unknown-protocol-version.json"),
    ),
    { status: "invalid", code: "mcp_capability_version_unsupported" },
  );
  assertEquals(
    parseMcpConfigurationRegistration(
      await loadFixture("registration/malformed.json"),
    ),
    { status: "invalid", code: "mcp_capability_invalid" },
  );
  assertEquals(
    parseMcpConfigurationRegistration(
      await loadFixture("registration/duplicate-transports.json"),
    ),
    { status: "invalid", code: "mcp_capability_invalid" },
  );
  assertEquals(
    parseMcpConfigurationRegistration(
      await loadFixture("registration/unknown-transport.json"),
    ),
    { status: "invalid", code: "mcp_capability_invalid" },
  );
  assertEquals(
    parseMcpConfigurationRegistration(
      await loadFixture("registration/empty-transports.json"),
    ),
    { status: "invalid", code: "mcp_capability_invalid" },
  );
  assertEquals(
    negotiateMcpConfiguration(await loadFixture("registration/omitted.json")),
    { status: "unsupported" },
  );
  assertEquals(
    negotiateMcpConfiguration(
      await loadFixture("registration/capability-without-handler.json"),
    ),
    { status: "disabled", code: "mcp_capability_invalid" },
  );
  assertEquals(
    negotiateMcpConfiguration(
      await loadFixture("registration/handler-without-capability.json"),
    ),
    { status: "disabled", code: "mcp_capability_invalid" },
  );
  assertEquals(
    negotiateMcpConfiguration(
      await loadFixture("registration/valid-http-v1.json"),
    ),
    { status: "enabled", capability: HTTP_V1 },
  );
});

Deno.test("shared receipt fixtures reject missing, duplicate, extra, and mismatched coverage", async () => {
  const tavilyCoverage = {
    generation: 4,
    desired: [{
      managedIdentity: "mcp-tavily",
      sourceRevisionId: "rev-tavily-1",
    }],
  };
  const valid = parseMcpConfigurationReceipt(
    await loadFixture("receipts/valid.json"),
  );
  assertEquals(valid, {
    ok: true,
    receipt: await loadFixture("receipts/valid.json"),
  });
  if (!valid.ok) {
    throw new Error("valid receipt must parse");
  }
  assertEquals(
    validateMcpConfigurationReceiptCoverage(valid.receipt, tavilyCoverage),
    undefined,
  );

  const coverageCases = [
    ["missing.json", "missing_managed_identity"],
    ["duplicate.json", "duplicate_managed_identity"],
    ["extra.json", "extra_managed_identity"],
    ["generation-mismatch.json", "generation_mismatch"],
    ["source-revision-mismatch.json", "source_revision_mismatch"],
  ] as const;
  for (const [name, code] of coverageCases) {
    const parsed = parseMcpConfigurationReceipt(
      await loadFixture(`receipts/${name}`),
    );
    if (!parsed.ok) {
      throw new Error(`${name} must parse as a receipt object`);
    }
    assertEquals(
      validateMcpConfigurationReceiptCoverage(parsed.receipt, tavilyCoverage),
      code,
    );
  }

  const duplicateNativeKey = parseMcpConfigurationReceipt(
    await loadFixture("receipts/duplicate-native-key.json"),
  );
  if (!duplicateNativeKey.ok) {
    throw new Error("duplicate-native-key.json must parse as a receipt object");
  }
  assertEquals(
    validateMcpConfigurationReceiptCoverage(duplicateNativeKey.receipt, {
      generation: 4,
      desired: [
        { managedIdentity: "mcp-tavily", sourceRevisionId: "rev-tavily-1" },
        { managedIdentity: "mcp-other", sourceRevisionId: "rev-other-1" },
      ],
    }),
    "duplicate_native_key",
  );

  const uppercase = structuredClone(
    await loadFixture("receipts/valid.json"),
  ) as {
    documentFingerprint: string;
    entries: { entryFingerprint: string }[];
  };
  uppercase.documentFingerprint =
    "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
  uppercase.entries[0].entryFingerprint =
    "sha256:BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB";
  assertEquals(parseMcpConfigurationReceipt(uppercase), {
    ok: true,
    receipt: await loadFixture("receipts/valid.json"),
  });

  assertEquals(
    parseMcpConfigurationReceipt(
      await loadFixture("receipts/illegal-fingerprint.json"),
    ),
    { ok: false, code: "illegal_fingerprint" },
  );
  assertEquals(
    parseMcpConfigurationReceipt(
      await loadFixture("receipts/locator-escape.json"),
    ),
    { ok: false, code: "locator_out_of_bounds" },
  );
});
