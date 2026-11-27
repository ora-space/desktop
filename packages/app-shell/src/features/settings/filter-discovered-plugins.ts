import type { InstalledPlugin } from "@ora/contracts";

/**
 * How the installed-plugin list is narrowed.
 *
 * Typed text searches fuzzily across every exposed field, while a navigation target names one
 * canonical id that must match exactly; keeping the two apart is what lets a jump land on a single
 * card even when another installed id extends the target as a prefix.
 */
export type InstalledPluginFilter =
  { kind: "search"; text: string } | { kind: "locate"; pluginId: string };

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
    [
      plugin.displayName,
      plugin.id,
      plugin.description,
      ...pluginSearchFields(plugin),
    ].some((value) => value.toLowerCase().includes(needle)),
  );
}

/** Narrows the installed list by the given filter, in backend order. */
export function filterInstalledPlugins(
  plugins: InstalledPlugin[],
  filter: InstalledPluginFilter,
): InstalledPlugin[] {
  switch (filter.kind) {
    case "search":
      return filterDiscoveredPlugins(plugins, filter.text);
    case "locate": {
      // Ids are compared the way search compares them, so a target spelled in another case still
      // lands on its card.
      const target = filter.pluginId.toLowerCase();
      return plugins.filter((plugin) => plugin.id.toLowerCase() === target);
    }
  }
}
