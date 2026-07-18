import { hasPlatformHostRenderer, renderPlatformHost } from "./platform-host-renderer";
import { usePlatform } from "./use-platform";
import type { PlatformLocale } from "./types";

/** Renders adapter-owned overlays inside the application's locale and theme provider tree. */
export function PlatformHost({ locale }: { locale: PlatformLocale }) {
  const adapter = usePlatform();
  return hasPlatformHostRenderer(adapter) ? adapter[renderPlatformHost](locale) : null;
}
