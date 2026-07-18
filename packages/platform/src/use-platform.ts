import { useContext } from "react";
import { PlatformContext } from "./platform-context-value";
import type { PlatformAdapter } from "./types";

/** Returns the active platform adapter and rejects missing host composition early. */
export function usePlatform(): PlatformAdapter {
  const adapter = useContext(PlatformContext);
  if (adapter === null) {
    throw new Error("usePlatform must be used within PlatformProvider");
  }

  return adapter;
}
