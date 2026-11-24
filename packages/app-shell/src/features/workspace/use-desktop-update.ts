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
    let observed = false;
    let unsubscribe: (() => void) | undefined;
    void updates
      .onStatus((next) => {
        observed = true;
        if (active) setStatus(next);
      })
      .then((stop) => {
        unsubscribe = stop;
        if (!active) stop();
      })
      // Read after subscribing, so no change is missed in between; any event is newer than a snapshot
      // that answers after it.
      .finally(() => {
        if (!active) return;
        return updates.getStatus().then((next) => {
          if (active && !observed) setStatus(next);
        });
      });
    return () => {
      active = false;
      unsubscribe?.();
    };
  }, [updates]);

  return { updates, status };
}
