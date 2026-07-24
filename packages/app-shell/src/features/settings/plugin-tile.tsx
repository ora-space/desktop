import { cn } from "@ora/ui";
import type { PluginEntry } from "./plugin-catalog";

const TILE_SIZES = {
  sm: { box: "size-8 rounded-lg", mark: "size-4" },
  md: { box: "size-10 rounded-xl", mark: "size-5" },
  lg: { box: "size-11 rounded-xl", mark: "size-6" },
  xl: { box: "size-14 rounded-2xl", mark: "size-7" },
} as const;

/**
 * The rounded, tinted square a plugin's mark sits in. Every surface — the installed
 * strip, the browse cards and the detail header — renders the same tile at a
 * different size so a plugin stays recognisable as the user moves between them.
 */
export function PluginTile({ plugin, size = "md", className }: { plugin: PluginEntry; size?: keyof typeof TILE_SIZES; className?: string }) {
  const Mark = plugin.mark;
  const tile = TILE_SIZES[size];
  return (
    <span className={cn("flex shrink-0 items-center justify-center ring-1 ring-border", tile.box, plugin.tint, className)}>
      <Mark className={tile.mark} />
    </span>
  );
}
