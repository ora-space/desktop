import { useEffect, useState } from "react";
import { useOptionalPlatform } from "../../platform/use-platform";
import type {
  DesktopUpdateCapability,
  DesktopUpdateStatus,
} from "../../platform/types";

/** Follows the native updater, or reports no capability on a host that cannot update itself. */
export function useDesktopUpdate(): {
  updates: DesktopUpdateCapability | undefined;
  status: DesktopUpdateStatus;
} {
  const updates = useOptionalPlatform()?.updates;
  const [status, setStatus] = useState<DesktopUpdateStatus>({
    kind: "current",
  });

  useEffect(() => {
    if (updates === undefined) return;
    let active = true;
    void updates.getStatus().then((next) => {
      if (active) setStatus(next);
    });
    let unsubscribe: (() => void) | undefined;
    void updates
      .onStatus((next) => {
        if (active) setStatus(next);
      })
      .then((stop) => {
        unsubscribe = stop;
        if (!active) stop();
      });
    return () => {
      active = false;
      unsubscribe?.();
    };
  }, [updates]);

  return { updates, status };
}
