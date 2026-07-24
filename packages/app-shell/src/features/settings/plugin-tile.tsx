import { cn } from "@ora/ui";
import type { PluginEntry } from "./plugin-catalog";

/**
 * The mark is drawn bare, so it carries the size the tinted plate used to. The box
 * keeps its original footprint to hold the surrounding layouts in alignment.
 */
const TILE_SIZES = {
  sm: { box: "size-8", mark: "size-5" },
  md: { box: "size-10", mark: "size-6" },
  lg: { box: "size-11", mark: "size-7" },
  xl: { box: "size-14", mark: "size-10" },
} as const;

/**
 * A plugin's brand mark, in its own colour and with no plate behind it. Every
 * surface — the installed strip, the browse cards and the detail header — renders
 * the same mark at a different size so a plugin stays recognisable as the user
 * moves between them.
 */
export function PluginTile({ plugin, size = "md", className }: { plugin: PluginEntry; size?: keyof typeof TILE_SIZES; className?: string }) {
  const Mark = plugin.mark;
  const tile = TILE_SIZES[size];
  return (
    <span className={cn("flex shrink-0 items-center justify-center", tile.box, plugin.tone, className)}>
      <Mark className={tile.mark} />
    </span>
  );
}
