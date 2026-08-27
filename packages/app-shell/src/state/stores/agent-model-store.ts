import type * as acp from "@agentclientprotocol/sdk";
import { create } from "zustand";

interface AgentModelState {
  /** The configuration options each agent reported most recently, keyed by that agent. */
  known: Record<string, acp.SessionConfigOption[] | undefined>;
  /** Records what one agent last reported, so the next surface opening on it renders at once. */
  remember: (
    agentCli: string,
    configOptions: acp.SessionConfigOption[],
  ) => void;
  /** Drops an agent's display cache before its plugin lifecycle is restarted. */
  forget: (agentCli: string) => void;
}

/**
 * Remembers what each agent offers, so opening a chat does not start blank.
 *
 * ACP reports a session's configuration options only as part of creating or
 * loading a session, so a new chat surface has to wait out a `session/new`
 * handshake before it can name a single model. That handshake is the slow part
 * of opening a chat, and it answers a question that barely changes: which models
 * an agent offers is a property of that agent's install and provider configuration,
 * not of the surface being opened. Keying on the agent identity alone is what lets one
 * surface's answer serve the next one's first paint.
 *
 * What is cached is a *display* answer, never an authoritative one. The real
 * handshake still runs, whatever it reports replaces this, and a cached list is
 * only ever shown while the session that owns the real one does not exist yet.
 * Nothing is chosen from it: applying a selection needs a session id, and until
 * the handshake produces one the list is offered to read rather than to act on.
 *
 * Deliberately unpersisted. It is worth exactly one app run — the first
 * handshake of the next one refills it — and nothing depends on it surviving.
 */
export const useAgentModelStore = create<AgentModelState>((set) => ({
  known: {},
  // Compared by reference on purpose. The warm-session query is pinned, so every
  // caller reading one surface's response hands over the same array; without this
  // the picker and the composer would take turns rewriting the same entry and
  // re-rendering each other.
  remember: (agentCli, configOptions) =>
    set((state) =>
      state.known[agentCli] === configOptions
        ? state
        : { known: { ...state.known, [agentCli]: configOptions } },
    ),
  forget: (agentCli) =>
    set((state) => {
      if (!(agentCli in state.known)) return state;
      const known = { ...state.known };
      delete known[agentCli];
      return { known };
    }),
}));
