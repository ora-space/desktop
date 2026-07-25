import { useEffect, useRef, useState } from "react";
import type { DashboardEndpoint, DashboardResolver } from "./types";

/** Hook state for resolving a dashboard iframe endpoint for one Ora session. */
interface DashboardEndpointState {
  endpoint: DashboardEndpoint | null;
  isLoading: boolean;
  error: string | null;
}

/** Empty state surfaced whenever the panel is closed or no session is selected. */
const EMPTY_STATE: DashboardEndpointState = {
  endpoint: null,
  isLoading: false,
  error: null,
};

/**
 * The session id the currently-resolved endpoint belongs to.
 *
 * When the user switches sessions, the old endpoint is stale (its URL still
 * carries the previous session id). Tracking the owning session lets the hook
 * treat a mismatch as "loading" so the panel shows a loading state instead of
 * briefly rendering the previous session's iframe.
 */
interface ResolvedState {
  sessionId: string | null;
  endpoint: DashboardEndpoint | null;
  error: string | null;
}

const INITIAL_RESOLVED: ResolvedState = {
  sessionId: null,
  endpoint: null,
  error: null,
};

/**
 * Resolves the dashboard endpoint for one Ora session id.
 *
 * The effect only kicks off an async resolve (and only mutates state from its
 * then/catch callbacks), so it never triggers a synchronous state update during
 * render. When the panel is closed or no session is selected, the hook returns a
 * derived empty state rather than resetting local state, avoiding stale-iframe
 * leaks without a setState-in-effect.
 */
export function useDashboardEndpoint(
  sessionId: string | null,
  resolve: DashboardResolver | null,
  open: boolean,
): DashboardEndpointState {
  // Tracks the last resolve this hook kicked off so a slow earlier resolve cannot
  // overwrite the result of a newer one after a session switch.
  const activeResolve = useRef<Promise<DashboardEndpoint> | null>(null);
  const [resolved, setResolved] = useState<ResolvedState>(INITIAL_RESOLVED);

  const canResolve = open && sessionId !== null && resolve !== null;

  useEffect(() => {
    if (!canResolve || !sessionId || !resolve) return;

    // Capture this resolve as the active one; a later switch supersedes it.
    const promise = resolve(sessionId);
    activeResolve.current = promise;

    let superseded = false;
    promise
      .then((endpoint) => {
        if (!superseded && activeResolve.current === promise) {
          setResolved({ sessionId, endpoint, error: null });
        }
      })
      .catch((error) => {
        if (!superseded && activeResolve.current === promise) {
          const message =
            error instanceof Error ? error.message : "Failed to resolve dashboard endpoint";
          setResolved({ sessionId, endpoint: null, error: message });
        }
      });

    return () => {
      // Mark the previous resolve as stale so late callbacks are ignored.
      superseded = true;
      if (activeResolve.current === promise) activeResolve.current = null;
    };
  }, [canResolve, sessionId, resolve]);

  // Derive the public state: nothing to resolve while closed/unselected.
  if (!canResolve) return EMPTY_STATE;
  // When the session switched, the previously-resolved endpoint is stale (its
  // URL carries the old session id). Treat that as loading so the panel shows a
  // loading state instead of briefly rendering the wrong session's iframe.
  const isStale = resolved.sessionId !== sessionId;
  return {
    endpoint: isStale ? null : resolved.endpoint,
    isLoading: isStale || (resolved.endpoint === null && resolved.error === null),
    error: isStale ? null : resolved.error,
  };
}
