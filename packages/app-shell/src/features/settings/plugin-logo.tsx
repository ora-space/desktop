import type { ComponentType } from "react";
import { IconPlug } from "@tabler/icons-react";

/**
 * Renders a plugin's own brand mark, shipped as the `logo.svg` inside its package and delivered
 * inline by the backend after security validation.
 *
 * The mark is drawn through an `<img>` rather than inlined into the DOM: an SVG referenced as an
 * image never runs scripts or loads external resources, so it stays inert even if a future
 * package slips something past validation. Packages without a logo fall back to a generic mark
 * so every row keeps the same shape; `fallback` lets a surface pick one that reads correctly for
 * what it is listing, such as an agent rather than a plugin in general.
 */
export function PluginLogoMark({
  logo,
  className,
  fallback: Fallback = IconPlug,
}: {
  logo: string | null | undefined;
  className?: string;
  fallback?: ComponentType<{ className?: string }>;
}) {
  return logo === null || logo === undefined ? (
    <Fallback className={className} />
  ) : (
    <img
      src={svgDataUrl(logo)}
      alt=""
      aria-hidden="true"
      className={className}
    />
  );
}

/** The settings list's fixed-size plugin mark, centred in the row's leading column. */
export function PluginLogo({ logo }: { logo: string | null }) {
  return (
    <span className="flex size-10 shrink-0 items-center justify-center text-muted-foreground">
      <PluginLogoMark logo={logo} className="size-6 object-contain" />
    </span>
  );
}

/**
 * Wraps SVG source in a `data:` URL. Percent-encoding rather than base64 keeps the markup
 * readable in devtools and avoids pulling in an encoder for what is already text.
 */
function svgDataUrl(svg: string) {
  return `data:image/svg+xml;charset=utf-8,${encodeURIComponent(svg)}`;
}
