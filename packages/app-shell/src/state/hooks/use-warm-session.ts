import { useQuery } from "@tanstack/react-query";
import { useStore } from "zustand";
import { useEffect } from "react";
import type { AgentCli, WarmSessionTarget } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { useChatStore } from "../../chat-store-context";
import { clientId } from "../client-id";
import { queryKeys } from "./query-keys";
import { useSessions } from "./use-sessions";

/** The provider session backing one chat surface, and whether it is still being opened. */
export interface WarmSession {
  /** The warm session's id, or `null` until this surface has one. */
  sessionId: string | null;
  /**
   * True only while this surface's handshake is in flight.
   *
   * A surface with nothing to warm and one whose handshake failed both report
   * `false`, so a caller can tell "not yet" from "never" — a null `sessionId`
   * alone cannot distinguish them.
   */
  isOpening: boolean;
}

/**
 * Opens the provider session that backs a chat surface before anything is sent.
 *
 * ACP reports a session's configuration options — the model list among them —
 * only as part of creating or loading a session, so a model cannot be chosen
 * until one exists. Warming here is what lets the composer show real models on
 * an empty chat, and it moves the agent handshake off the send path.
 *
 * The id is `null` when there is nothing to warm: a persisted session is already
 * selected (its options arrive with `session/load`), or no project is chosen.
 */
export function useWarmSession(
  selection: { projectId: string | null; taskId: string | null; sessionId: string | null },
  agentCli: AgentCli,
): WarmSession {
  const client = useContractsClient();
  const chatStore = useChatStore();
  const setConfigOptions = useStore(chatStore, (state) => state.setConfigOptions);
  const { data: sessions = [] } = useSessions();
  // Selection can already point at a session that was never persisted — a chat
  // whose attach failed, for one — and that surface still needs a warm session
  // to retry with. Only a session the backend actually stored ends warming.
  const isPersisted =
    selection.sessionId !== null
    && sessions.some((session) => session.id === selection.sessionId);
  const target = isPersisted ? null : warmTarget(selection);

  // The backend keys warm sessions by exactly these values, so the same surface
  // always resolves to the same session and repeated calls are cache hits rather
  // than new provider sessions.
  const { data, isLoading } = useQuery({
    queryKey: queryKeys.warmSession(target, agentCli),
    enabled: target !== null,
    queryFn: () =>
      client.session.warm({ target: target!, agentCli, clientId: clientId() }),
    // A warm session is owned by the backend and only changes when this client
    // asks it to, so re-fetching it on remount would only risk creating another.
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
  });

  useEffect(() => {
    if (data === undefined) return;
    // This response is a snapshot of the handshake, and the query holding it is
    // pinned (`staleTime`/`gcTime: Infinity`) so every remount hands back the
    // same one. The store is where the session's options actually live — a model
    // picked since the handshake exists only there — so this seeds a session the
    // store has not heard of yet and never replays over one it has. Without that,
    // returning to a still-warm surface would restore its opening model.
    if (chatStore.getState().conversations[data.sessionId] !== undefined) return;
    setConfigOptions(data.sessionId, data.configOptions);
  }, [chatStore, data, setConfigOptions]);

  // `isLoading` is `isPending && isFetching`, so a disabled query (nothing to
  // warm) and a failed handshake — which does not retry — both read as false.
  // Only a request actually in flight counts as still opening.
  return { sessionId: data?.sessionId ?? null, isOpening: isLoading };
}

/** Derives what a chat surface should warm against, or `null` when nothing should. */
function warmTarget(selection: {
  projectId: string | null;
  taskId: string | null;
}): WarmSessionTarget | null {
  if (selection.taskId !== null) return { type: "task", taskId: selection.taskId };
  if (selection.projectId !== null) {
    // A direct chat creates its Task in project-root mode when the first message
    // is sent, so the project root is already the directory it will resolve to.
    return { type: "projectRoot", projectId: selection.projectId };
  }
  return null;
}
