import type { ReactNode } from "react";
import type { PlatformLocale } from "./types";

export const renderPlatformHost = Symbol("renderPlatformHost");

/** Internal capability implemented only by adapters that need React-owned host UI. */
export interface PlatformHostRenderer {
  [renderPlatformHost](locale: PlatformLocale): ReactNode;
}

/** Detects the optional host renderer without adding UI methods to PlatformAdapter. */
export function hasPlatformHostRenderer(value: object): value is PlatformHostRenderer {
  return renderPlatformHost in value;
}
