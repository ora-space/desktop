import type * as acp from "@agentclientprotocol/sdk";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useStore } from "zustand";
import { useEffect } from "react";
import type {
  AgentCli,
  WarmSessionResponse,
  WarmSessionTarget,
} from "@ora/contracts";
import type { ChatStore } from "@ora/chat";
import { useContractsClient } from "../../contracts-client-context";
import { useChatStore } from "../../chat-store-context";
import { queryKeys } from "./query-keys";
import { useSessions } from "./use-sessions";
import { usePendingSwitch } from "../stores/pending-agent-store";
import { useAgentModelStore } from "../stores/agent-model-store";

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
  /**
   * Resolves this surface's warm session id, waiting out a handshake that is
   * still open, and rejects if that handshake fails.
   *
   * The send path needs the id itself rather than a render-time snapshot of it:
   * a message sent while the handshake is in flight reads `sessionId` as `null`
   * and would otherwise have nowhere to go. Resolves to `null` only when there
   * is genuinely nothing to warm.
   */
  ensureSessionId: () => Promise<string | null>;
}

/**
 * Seeds a warm session's options into the chat store, but never over a
 * conversation the store already knows.
 *
 * The handshake response is a snapshot, and the query holding it is pinned
 * (`staleTime`/`gcTime: Infinity`) so every remount hands back the same one. The
 * store is where a session's options actually live — a model picked since the
 * handshake exists only there — so replaying that snapshot over a live
 * conversation would restore its opening model.
 */
function seedConfigOptions(
  chatStore: ChatStore,
  setConfigOptions: (
    oraSessionId: string,
    configOptions: acp.SessionConfigOption[],
  ) => void,
  warmed: WarmSessionResponse,
): void {
  if (chatStore.getState().conversations[warmed.sessionId] !== undefined)
    return;
  setConfigOptions(warmed.sessionId, warmed.configOptions);
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
  selection: {
    projectId: string | null;
    taskId: string | null;
    sessionId: string | null;
  },
  agentCli: AgentCli,
): WarmSession {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const chatStore = useChatStore();
  const setConfigOptions = useStore(
    chatStore,
    (state) => state.setConfigOptions,
  );
  const { data: sessions = [] } = useSessions();
  const pendingSwitch = usePendingSwitch(selection.sessionId);
  const rememberModels = useAgentModelStore((state) => state.remember);
  // Selection can already point at a session that was never persisted — a chat
  // whose attach failed, for one — and that surface still needs a warm session
  // to retry with. Only a session the backend actually stored ends warming.
  const isPersisted =
    selection.sessionId !== null &&
    sessions.some((session) => session.id === selection.sessionId);
  // A persisted session normally has nothing to warm: its options arrive with
  // `session/load`. A pending move is the exception — the CLI it is moving to
  // has not handshaken here yet, and warming is the only way to show its models
  // before the move is paid for. The backend claims this very session when the
  // move commits, so the model chosen on it survives into the rebind.
  const target =
    isPersisted && pendingSwitch === undefined ? null : warmTarget(selection);

  // The backend keys warm sessions by exactly these values, so the same surface
  // always resolves to the same session and repeated calls are cache hits rather
  // than new provider sessions. The subscribing query below and `ensureSessionId`
  // share this definition so they address one cache entry: two spellings of the
  // same request would hand the backend two provider sessions for one surface.
  const queryOptions = {
    queryKey: queryKeys.warmSession(target, agentCli),
    queryFn: () => client.session.warm({ target: target!, agentCli }),
    // A warm session is owned by the backend and only changes when this client
    // asks it to, so re-fetching it on remount would only risk creating another.
    staleTime: Infinity,
    gcTime: Infinity,
    retry: false,
  };
  const { data, isLoading } = useQuery({
    ...queryOptions,
    enabled: target !== null,
  });

  const ensureSessionId = async (): Promise<string | null> => {
    if (target === null) return null;
    // Goes through the cache rather than calling the backend directly, so a
    // handshake this hook already started is awaited instead of duplicated.
    // Because warming does not retry, a previously failed one is retried here.
    const response = await queryClient.ensureQueryData(queryOptions);
    // Seeded here and not left to the effect below: a caller that was waiting on
    // this opens the conversation the moment it resolves, and the effect — which
    // runs a render later and never writes over a conversation the store already
    // knows — would find it there and skip, leaving the session with no models.
    seedConfigOptions(chatStore, setConfigOptions, response);
    // Recorded here for the same reason, against a narrower race: a send
    // resolves this and then points the workspace at the new session, so this
    // hook can be unmounted before the effect below ever runs and the handshake
    // the user waited on would be the one nothing remembered.
    rememberModels(agentCli, response.configOptions);
    return response.sessionId;
  };

  useEffect(() => {
    if (data === undefined) return;
    seedConfigOptions(chatStore, setConfigOptions, data);
  }, [chatStore, data, setConfigOptions]);

  // Kept out of the effect above, which stops at a session the store already
  // knows. This one has to run for every handshake: what it records is scoped to
  // the CLI, not to the session, and it is the only thing that lets the *next*
  // surface opening on this CLI paint a model list before its own handshake.
  useEffect(() => {
    if (data === undefined) return;
    rememberModels(agentCli, data.configOptions);
  }, [agentCli, data, rememberModels]);

  // `isLoading` is `isPending && isFetching`, so a disabled query (nothing to
  // warm) and a failed handshake — which does not retry — both read as false.
  // Only a request actually in flight counts as still opening.
  return {
    sessionId: data?.sessionId ?? null,
    isOpening: isLoading,
    ensureSessionId,
  };
}

/** Derives what a chat surface should warm against, or `null` when nothing should. */
function warmTarget(selection: {
  projectId: string | null;
  taskId: string | null;
}): WarmSessionTarget | null {
  if (selection.taskId !== null)
    return { type: "task", taskId: selection.taskId };
  if (selection.projectId !== null) {
    // A direct chat creates its Task in project-root mode when the first message
    // is sent, so the project root is already the directory it will resolve to.
    return { type: "projectRoot", projectId: selection.projectId };
  }
  return null;
}

/**
 * Reduces a warm target to the string key callers use to scope per-target UI state.
 *
 * Shares `warmTarget`'s precedence (task before project root) so a value keyed
 * off this always lines up with the warm session it describes — most directly,
 * which agent a not-yet-started chat surface is currently offering.
 */
export function warmTargetKey(selection: {
  projectId: string | null;
  taskId: string | null;
}): string | null {
  const target = warmTarget(selection);
  if (target === null) return null;
  return target.type === "task"
    ? `task:${target.taskId}`
    : `project:${target.projectId}`;
}
