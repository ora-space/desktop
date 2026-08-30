import type { AgentMcpRenderResult, McpServerRef } from "./agent.ts";

/** The OpenCode config schema Ora writes first so OpenCode never rewrites this Ora-owned file. */
const OPENCODE_CONFIG_SCHEMA = "https://opencode.ai/config.json";

/**
 * Derives the ADR-0015 collision-resistant MCP key for one server from its canonical plugin id.
 *
 * The key is `"ora__" + identifier.split(".").join("__")`: the `identifier` is the post-namespace
 * half of the canonical plugin id (e.g. `ora-space.tavily-search` for `official/ora-space.tavily-search`),
 * and each `.` publisher/tool separator becomes a `__` so the key reads as `ora__<publisher>__<tool>`
 * (here, `ora__ora-space__tavily-search`). The `official` install namespace is carried on the wire
 * for host-side selection identity but is intentionally not part of the key: ADR-0015 treats same-key
 * shadow across unobservable OpenCode layers as a known, reversible first-release limitation rather
 * than claiming proof against it, and Ora separately scans Workspace-visible layers and fails on a
 * known collision.
 */
function mcpServerKey(identifier: string): string {
  return `ora__${identifier.split(".").join("__")}`;
}

/**
 * Renders the complete OpenCode `.opencode/opencode.jsonc` body from a plaintext-free desired set.
 *
 * Each server becomes one `mcp[<key>]` entry carrying `type: "remote"`, the canonical URL,
 * `enabled: true`, and headers whose values are `prefix + "{env:" + envVar + "}" + suffix`. The
 * reference is the env-var NAME (never the bound Setting value), so a renderer handed only this type
 * cannot leak a key it was never given; OpenCode substitutes `{env:VAR}` from the process
 * environment before JSONC parsing, so the persisted file never carries a secret. The document carries
 * `$schema` first so OpenCode never rewrites this Ora-owned file to add one.
 *
 * The returned `digest` is `sha256:` + lowercase hex over the UTF-8 bytes, computed via Web Crypto.
 * Ora's Rust host recomputes the same digest over the bytes with `Digest::sha256` and rejects any
 * mismatch as `McpRenderError::Ipc`, so a renderer cannot vouch for content it did not produce —
 * the host never trusts a digest it did not recompute.
 */
export function renderOpenCodeMcpFile(
  servers: readonly McpServerRef[],
): Promise<AgentMcpRenderResult> {
  const mcp: Record<string, unknown> = {};
  for (const server of servers) {
    const headers: Record<string, string> = {};
    for (const header of server.headers) {
      headers[header.name] =
        `${header.prefix}{env:${header.envVar}}${header.suffix}`;
    }
    mcp[mcpServerKey(server.identifier)] = {
      type: "remote",
      url: server.url,
      enabled: true,
      headers,
    };
  }
  const bytes = JSON.stringify({
    "$schema": OPENCODE_CONFIG_SCHEMA,
    mcp,
  });
  return digestOf(bytes).then((digest) => ({ bytes, digest }));
}

/**
 * Computes the `sha256:` + lowercase-hex digest Ora's host rechecks, via the standard Web Crypto.
 *
 * The `sha256:` prefix and 64-char lowercase-hex body match `ora_effect::Digest::sha256(...).as_str()`
 * exactly, which is what the host compares the returned `digest` against.
 */
async function digestOf(bytes: string): Promise<string> {
  const hashed = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(bytes),
  );
  const hex = Array.from(new Uint8Array(hashed))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
  return `sha256:${hex}`;
}
