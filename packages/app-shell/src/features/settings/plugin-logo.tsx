import { IconPlug } from "@tabler/icons-react";

/**
 * Renders a plugin's own brand mark, shipped as the `logo.svg` inside its package and delivered
 * inline by the backend after security validation.
 *
 * The mark is drawn through an `<img>` rather than inlined into the DOM: an SVG referenced as an
 * image never runs scripts or loads external resources, so it stays inert even if a future
 * package slips something past validation. Plugins without a logo fall back to the generic plug
 * mark so every row keeps the same shape.
 */
export function PluginLogo({ logo }: { logo: string | null }) {
  return (
    <span className="flex size-10 shrink-0 items-center justify-center text-muted-foreground">
      {logo === null ? (
        <IconPlug className="size-6" />
      ) : (
        <img
          src={svgDataUrl(logo)}
          alt=""
          aria-hidden="true"
          className="size-6 object-contain"
        />
      )}
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
