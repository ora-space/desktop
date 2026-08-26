import type { InstalledPlugin } from "@ora/contracts";

/** Collects the kind-specific fields that installed-plugin search should match. */
function pluginSearchFields(plugin: InstalledPlugin): string[] {
  if (plugin.kind === "agent") {
    return [plugin.agentDisplayName];
  }
  if (plugin.kind === "workbench" || plugin.kind === "webview") {
    return [plugin.title];
  }
  return [];
}

/** Filters discovered packages across every field exposed by installed-plugin search. */
export function filterDiscoveredPlugins(
  plugins: InstalledPlugin[],
  query: string,
): InstalledPlugin[] {
  const needle = query.trim().toLowerCase();
  if (!needle) return plugins;
  return plugins.filter((plugin) =>
    [plugin.displayName, plugin.id, plugin.description, ...pluginSearchFields(plugin)].some(
      (value) => value.toLowerCase().includes(needle),
    ),
  );
}
