import type { McpServerRef } from "../src/agent.ts";
import { renderOpenCodeMcpFile } from "../src/opencode-mcp.ts";

/**
 * The complete OpenCode `.opencode/opencode.jsonc` body the renderer must emit for one
 * Tavily-shaped server, taken verbatim from the §5.8 illustrative file in compact-JSON form.
 * The host prepends the `// ora-managed-mcp <digest>` ownership marker line separately, so the
 * renderer emits only this body — never the marker.
 *
 * The key `ora__ora-space__tavily-search` is the ADR-0015 collision-resistant derivation of the
 * canonical plugin id: `"ora__" + identifier.split(".").join("__")` over `ora-space.tavily-search`.
 * The `official` install namespace is carried for host-side selection identity but intentionally
 * omitted from the key (ADR-0015 treats cross-namespace shadow in unobservable layers as a known
 * first-release limitation rather than proof against it).
 */
const TAVILY_BODY =
  `{"$schema":"https://opencode.ai/config.json","mcp":{"ora__ora-space__tavily-search":{"type":"remote","url":"https://mcp.tavily.com/mcp","enabled":true,"headers":{"Authorization":"Bearer {env:ORA_MCP_TAVILY_API_KEY}"}}}}`;

/**
 * sha256 of TAVILY_BODY computed with coreutils `sha256sum` (a C implementation), independent of
 * the Deno Web Crypto the renderer uses, so the digest assertion cannot pass by construction. Ora's
 * Rust host recomputes `sha256:` + lowercase hex over the returned bytes via `Digest::sha256` and
 * rejects any mismatch (`McpRenderError::Ipc`), so the renderer must emit this exact digest to
 * survive host validation at all.
 */
const TAVILY_DIGEST =
  "sha256:b660bad86a169e83b389056a6db36a1cea5ec4d284bdde75415178340478b803";

/** Builds the Tavily server exactly as the host hands it to the renderer. */
function tavilyServer(overrides: Partial<McpServerRef> = {}): McpServerRef {
  return {
    namespace: "official",
    identifier: "ora-space.tavily-search",
    version: "1.0.0",
    definitionDigest: "deadbeef",
    revision: 1,
    url: "https://mcp.tavily.com/mcp",
    headers: [{
      name: "Authorization",
      envVar: "ORA_MCP_TAVILY_API_KEY",
      prefix: "Bearer ",
      suffix: "",
    }],
    ...overrides,
  };
}

/** Compares JSON-compatible values without a Node compatibility dependency. */
function assertEquals(actual: unknown, expected: unknown): void {
  const actualJson = JSON.stringify(actual);
  const expectedJson = JSON.stringify(expected);
  if (actualJson !== expectedJson) {
    throw new Error(`Expected ${expectedJson}, received ${actualJson}`);
  }
}

Deno.test("renderOpenCodeMcpFile emits the OpenCode remote + env-ref shape for one server", async () => {
  const result = await renderOpenCodeMcpFile([tavilyServer()]);
  assertEquals(result.bytes, TAVILY_BODY);
  assertEquals(result.digest, TAVILY_DIGEST);
});

Deno.test("renders one mcp entry per server in request order with collision-resistant keys", async () => {
  const result = await renderOpenCodeMcpFile([
    tavilyServer(),
    {
      namespace: "official",
      identifier: "ora-space.example-tools",
      version: "2.0.0",
      definitionDigest: "cafef00d",
      revision: 3,
      url: "https://mcp.example.com/mcp",
      headers: [{
        name: "X-Api-Key",
        envVar: "ORA_EXAMPLE_KEY",
        prefix: "",
        suffix: "",
      }],
    },
  ]);
  assertEquals(
    result.bytes,
    `{"$schema":"https://opencode.ai/config.json","mcp":{"ora__ora-space__tavily-search":{"type":"remote","url":"https://mcp.tavily.com/mcp","enabled":true,"headers":{"Authorization":"Bearer {env:ORA_MCP_TAVILY_API_KEY}"}},"ora__ora-space__example-tools":{"type":"remote","url":"https://mcp.example.com/mcp","enabled":true,"headers":{"X-Api-Key":"{env:ORA_EXAMPLE_KEY}"}}}}`,
  );
});

Deno.test("composes each header value from prefix and suffix around the env reference", async () => {
  const result = await renderOpenCodeMcpFile([{
    namespace: "official",
    identifier: "ora-space.multi",
    version: "1.0.0",
    definitionDigest: "deadbeef",
    revision: 1,
    url: "https://mcp.example.com/mcp",
    headers: [
      {
        name: "Authorization",
        envVar: "ORA_KEY",
        prefix: "Token ",
        suffix: ", type=search",
      },
      { name: "X-Trace", envVar: "ORA_BARE", prefix: "", suffix: "" },
    ],
  }]);
  assertEquals(
    result.bytes,
    `{"$schema":"https://opencode.ai/config.json","mcp":{"ora__ora-space__multi":{"type":"remote","url":"https://mcp.example.com/mcp","enabled":true,"headers":{"Authorization":"Token {env:ORA_KEY}, type=search","X-Trace":"{env:ORA_BARE}"}}}}`,
  );
});

Deno.test("renders an empty mcp map when no server is desired", async () => {
  const result = await renderOpenCodeMcpFile([]);
  assertEquals(
    result.bytes,
    `{"$schema":"https://opencode.ai/config.json","mcp":{}}`,
  );
  // The digest still resolves (sha256 of the empty-mcp body) so the host marker stays well-formed.
  assertEquals(result.digest.startsWith("sha256:"), true);
  assertEquals(result.digest.length, "sha256:".length + 64);
});
