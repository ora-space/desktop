import type { ReactNode } from "react";
import { PlatformContext } from "./platform-context-value";
import type { PlatformAdapter } from "./types";

/** Makes the explicitly injected platform adapter available to application feature components. */
export function PlatformProvider({
  adapter,
  children,
}: {
  adapter: PlatformAdapter;
  children: ReactNode;
}) {
  return <PlatformContext.Provider value={adapter}>{children}</PlatformContext.Provider>;
}
