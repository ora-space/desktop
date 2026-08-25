import type { InstalledPlugin } from "@ora/contracts";

/**
 * One openable surface, carrying the plugin identity the host needs to address it and the kind
 * so the launcher can hint "local page" versus "external site".
 *
 * The content source (asset root or external origin) is deliberately absent: the host resolves
 * it from the installed manifest when opening, so the launcher treats every surface the same way.
 */
export type SurfaceDefinitionRef = {
  pluginId: string;
  kind: "workbench" | "webview";
  title: string;
  pluginDisplayName: string;
};

/**
 * Lists the surface of every enabled workbench or webview plugin in a stable menu order.
 *
 * Each such plugin contributes exactly one surface. Disabled plugins are excluded so that
 * disabling a plugin removes its entry without a separate lifecycle hook; ordering by plugin
 * name then title keeps the menu independent of backend snapshot order.
 */
export function listSurfaceDefinitions(
  plugins: readonly InstalledPlugin[],
): SurfaceDefinitionRef[] {
  const refs: SurfaceDefinitionRef[] = [];
  for (const plugin of plugins) {
    if (!plugin.enabled) continue;
    if (plugin.kind !== "workbench" && plugin.kind !== "webview") continue;
    refs.push({
      pluginId: plugin.id,
      kind: plugin.kind,
      title: plugin.title,
      pluginDisplayName: plugin.displayName,
    });
  }
  return refs.sort(
    (a, b) =>
      a.pluginDisplayName.localeCompare(b.pluginDisplayName) ||
      a.title.localeCompare(b.title),
  );
}
