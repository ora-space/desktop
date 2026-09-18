import type { QueryClient } from "@tanstack/react-query";

/**
 * Cache identity owned by Host MCP health.
 *
 * A view is keyed by the Session cwd it was resolved against; the plugin card uses the empty
 * (`null`) view. UI code selects a view, never a raw query-key string.
 */
export const mcpHealthKeys = {
  /** Every health view, invalidated when one plugin's identity or result changed. */
  all: ["mcp-health"] as const,
  /** One card (`null`) or Session (`cwd`) view. */
  view: (cwd: string | null) => ["mcp-health", cwd] as const,
};

/**
 * Refreshes every MCP health view after a probe settled, an identity was invalidated, or a plugin
 * was uninstalled. The `McpHealthChanged` event names only the plugin, and the same plugin may
 * appear in several Session views, so the whole family is invalidated rather than guessing which
 * view changed.
 */
export function invalidateMcpHealth(queryClient: QueryClient): Promise<void> {
  return queryClient.invalidateQueries({ queryKey: mcpHealthKeys.all });
}
