import type { InstalledPlugin } from "@ora/contracts";

/**
 * One openable surface, flattened with the plugin identity the host needs to address it.
 *
 * The content source (remote site or package-shipped panel) is deliberately absent: the
 * host resolves it from the installed manifest when opening, so the launcher treats every
 * surface the same way.
 */
export type SurfaceDefinitionRef = {
  pluginId: string;
  surfaceId: string;
  title: string;
  pluginDisplayName: string;
};

/**
 * Lists the surfaces of enabled ui plugins in a stable menu order.
 *
 * Disabled plugins are excluded so that disabling a plugin removes its entries without a
 * separate lifecycle hook; ordering by plugin name then title keeps the menu independent of
 * backend snapshot order.
 */
export function listSurfaceDefinitions(
  plugins: readonly InstalledPlugin[],
): SurfaceDefinitionRef[] {
  const refs: SurfaceDefinitionRef[] = [];
  for (const plugin of plugins) {
    if (!plugin.enabled || plugin.kind !== "ui") continue;
    for (const surface of plugin.surfaces) {
      refs.push({
        pluginId: plugin.id,
        surfaceId: surface.id,
        title: surface.title,
        pluginDisplayName: plugin.displayName,
      });
    }
  }
  return refs.sort(
    (a, b) =>
      a.pluginDisplayName.localeCompare(b.pluginDisplayName) ||
      a.title.localeCompare(b.title),
  );
}
