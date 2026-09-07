import type { Session } from "@ora/contracts";
import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { describe, expect, it, vi } from "vitest";
import {
  cacheDeletedSession,
  cacheRenamedSession,
  refetchSessions,
  sessionKeys,
} from "./sessions";

const session: Session = {
  id: "session-1",
  workspaceId: "workspace-1",
  agentRef: "agent-a",
  status: "running",
  title: "Before",
  historyState: { type: "writable" },
};

describe("session list cache policy", () => {
  it("applies rename without refetch, but actively refetches after an event gap", async () => {
    const client = new QueryClient();
    client.setQueryData(sessionKeys.sessions, [session]);
    const authoritative = { ...session, title: "From the server" };
    const fetch = vi.fn(async () => [authoritative]);
    const observer = new QueryObserver(client, {
      queryKey: sessionKeys.sessions,
      queryFn: fetch,
      staleTime: Infinity,
    });
    const unsubscribe = observer.subscribe(() => {});
    const renamed = { ...session, title: "Renamed" };
    cacheRenamedSession(client, renamed);
    expect(client.getQueryData(sessionKeys.sessions)).toEqual([renamed]);
    expect(fetch).not.toHaveBeenCalled();
    expect(client.getQueryState(sessionKeys.sessions)?.isInvalidated).toBe(
      true,
    );
    await refetchSessions(client);
    expect(fetch).toHaveBeenCalledOnce();
    expect(client.getQueryData(sessionKeys.sessions)).toEqual([authoritative]);
    unsubscribe();
    client.clear();
  });

  it.each(["immediate", "defer"] as const)(
    "scrubs a deleted session with %s list synchronization",
    (sync) => {
      const client = new QueryClient();
      const survivor = { ...session, id: "session-2" };
      client.setQueryData(sessionKeys.sessions, [session, survivor]);
      cacheDeletedSession(client, session.id, sync);
      expect(client.getQueryData(sessionKeys.sessions)).toEqual([survivor]);
      expect(client.getQueryState(sessionKeys.sessions)?.isInvalidated).toBe(
        sync === "immediate",
      );
      client.clear();
    },
  );
});
