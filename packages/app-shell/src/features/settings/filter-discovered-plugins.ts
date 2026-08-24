import type { InstalledPlugin } from "@ora/contracts";

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
      ...(plugin.kind === "agent" ? [plugin.agentDisplayName] : [plugin.title]),
    ].some((value) => value.toLowerCase().includes(needle)),
  );
}
