import type { KnownAgentCli } from "../../features/chat/model-catalog";
import { useSettingsStore } from "../stores/settings-store";
import { usePendingAgentStore } from "../stores/pending-agent-store";
import { warmTargetKey } from "./use-warm-session";
import { useSessions } from "./use-sessions";

/** The selection legs that decide which agent a chat surface is pointing at. */
interface AgentSelection {
  projectId: string | null;
  taskId: string | null;
  sessionId: string | null;
}

/**
 * Resolves which agent CLI a chat surface is currently pointing at.
 *
 * The composer and the model picker each warm a session against this answer, and
 * a warm session's identity includes the CLI — so two call sites that computed it
 * differently would build a second provider session and leave the picker offering
 * models the composer is not pointing at. Owning the whole precedence chain in
 * one place is what keeps them from drifting apart; callers must not re-derive
 * any part of it.
 *
 * A pending switch outranks the session's own binding. The user has chosen to
 * move this conversation, and everything on screen must already describe the
 * agent it is moving to, even though the binding itself does not change until
 * the next message is sent.
 *
 * With no pending move, a persisted session runs on whatever the backend has it
 * bound to, which is not necessarily the stored default — that only decides what
 * the *next* surface opens on. Before a session row exists there is nothing
 * bound, so the pick recorded for this exact target answers instead; reading the
 * shared default directly would let picking an agent for one not-yet-started chat
 * repaint every other one the moment it is visited.
 *
 * A binding or preference is reported as written even when that agent can no
 * longer be reached. Agent availability is allowed to change while a surface is
 * open, and using the first available agent as a fallback would silently change
 * the user's selection when a plugin is enabled or disabled. A surface that has
 * never chosen an agent therefore resolves to `null` instead of inventing one.
 */
export function useTargetAgentCli(
  selection: AgentSelection,
): KnownAgentCli | null {
  const defaultAgentCli = useSettingsStore((state) => state.settings.agentCli);
  const { data: sessions = [] } = useSessions();
  const targetKey = warmTargetKey(selection);
  const pendingSwitch = usePendingAgentStore((state) =>
    selection.sessionId === null
      ? undefined
      : state.switches[selection.sessionId],
  );
  const pickedForTarget = usePendingAgentStore((state) =>
    targetKey === null ? undefined : state.selections[targetKey],
  );
  // The wire carries any installed agent's identity as a plain string, but the picker this
  // hook drives only labels the closed set it knows, so a bound session is assumed to name one
  // of them. A binding onto an agent outside that set is still reported here — it is what the
  // session actually runs on — and simply has no entry to render against.
  const boundAgentCli = sessions.find(
    (session) => session.id === selection.sessionId,
  )?.agentRef as KnownAgentCli | undefined;
  if (pendingSwitch !== undefined) return pendingSwitch;
  if (boundAgentCli !== undefined) return boundAgentCli;
  return pickedForTarget ?? defaultAgentCli;
}
