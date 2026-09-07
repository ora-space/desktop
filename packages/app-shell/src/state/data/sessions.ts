import type { Session } from "@ora/contracts";
import type { QueryClient } from "@tanstack/react-query";

/** Cache identity owned by sessions data; consumers never repeat its tuples. */
export const sessionKeys = {
  sessions: ["sessions"] as const,
};

/**
 * How list caches should sync after a mutation.
 *
 * - `immediate`: patch and mark the list stale with an active refetch — use for
 *   standalone deletes the user just confirmed.
 * - `defer`: patch only; a parent cascade will invalidate once at the end so
 *   N child deletes do not thrash the sidebar N times.
 */
export type WorkspaceListSync = "immediate" | "defer";

/** Makes restored history writability visible immediately, then refreshes the session list. */
export function cacheResumedSession(
  queryClient: QueryClient,
  session: Session,
) {
  queryClient.setQueryData<Session[]>(sessionKeys.sessions, (current) =>
    (current ?? []).map((candidate) =>
      candidate.id === session.id ? session : candidate,
    ),
  );
  queryClient.invalidateQueries({ queryKey: sessionKeys.sessions });
}

/** Removes a session immediately; parent cascades defer their final list refresh. */
export function cacheDeletedSession(
  queryClient: QueryClient,
  sessionId: string,
  listSync: WorkspaceListSync,
) {
  queryClient.setQueryData<Session[]>(sessionKeys.sessions, (current) =>
    (current ?? []).filter((session) => session.id !== sessionId),
  );
  if (listSync === "immediate") {
    void invalidateSessions(queryClient);
  }
}

/** Applies a title response without refetching every subscriber. */
export function cacheRenamedSession(
  queryClient: QueryClient,
  session: Session,
) {
  // Replace with a new object so React Query cannot structural-share the
  // previous cache entry when the transport returns the same session reference.
  queryClient.setQueryData<Session[]>(sessionKeys.sessions, (current) =>
    (current ?? []).map((candidate) =>
      candidate.id === session.id ? { ...session } : candidate,
    ),
  );
  void queryClient.invalidateQueries({
    queryKey: sessionKeys.sessions,
    refetchType: "none",
  });
}

/** Refetches authoritative session state after a gap in app-event delivery. */
export function refetchSessions(queryClient: QueryClient) {
  return queryClient.refetchQueries({ queryKey: sessionKeys.sessions });
}

/** Marks session projections stale after a confirmed mutation or title event. */
export function invalidateSessions(queryClient: QueryClient) {
  return queryClient.invalidateQueries({ queryKey: sessionKeys.sessions });
}
