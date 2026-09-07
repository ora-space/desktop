import { type Session } from "@ora/contracts";
import type { TestHandlers } from "../contracts-transport";
import { nextId } from "./records";
import {
  createAgentModelMemory,
  type AgentModelMemoryState,
} from "./agent-models";

/** Mutable records owned by the sessions test adapter. */
export interface SessionMemoryState extends AgentModelMemoryState {
  sessions: Session[];
}

/** Creates an independent sessions memory fixture. */
export function createSessionMemory(): SessionMemoryState {
  return { sessions: [], ...createAgentModelMemory() };
}

/** Registers only the sessions operations explicitly requested by a fixture. */
export function sessionHandlers(state: SessionMemoryState) {
  return {
    listSessions: async () => ({ sessions: [...state.sessions] }),
    getSession: async (req) => ({
      session: state.sessions.find((s) => s.id === req.sessionId)!,
    }),
    startSession: async (req) => {
      const sessionId = nextId("s", state.sessions.length);
      const perCli = state.agentModelsByCli?.[req.agentRef];
      const configOptions = structuredClone(
        perCli === undefined ? state.configOptions : (perCli ?? []),
      );
      if (req.model !== null) {
        const option = configOptions.find(
          (candidate) =>
            candidate.type === "select" && candidate.category === "model",
        );
        if (option?.type === "select") option.currentValue = req.model;
      }
      const session: Session = {
        id: sessionId,
        workspaceId: req.workspaceId,
        agentRef: req.agentRef,
        status: "running",
        title: null,
        historyState: { type: "writable" },
      };
      state.sessions.push(session);
      return { session, availableCommands: [], configOptions };
    },
    setSessionConfig: async () => ({ configOptions: state.configOptions }),
    switchSessionAgent: async (req) => {
      const session = state.sessions.find(
        (candidate) => candidate.id === req.sessionId,
      )!;
      session.agentRef = req.agentRef;
      return {
        session,
        availableCommands: [],
        configOptions: state.configOptions,
      };
    },
    resumeSessionHistory: async (req) => {
      const session = state.sessions.find(
        (candidate) => candidate.id === req.sessionId,
      )!;
      session.historyState = { type: "writable" };
      return { session };
    },
    loadSession: async function* () {
      yield { type: "completed" as const };
    },
    promptSession: async function* () {
      yield { type: "completed" as const, stopReason: "end_turn" as const };
    },
    respondToSessionPermission: async () => ({}),
    cancelSessionPrompt: async () => ({}),
    stopSession: async (req) => {
      const session = state.sessions.find(
        (candidate) => candidate.id === req.sessionId,
      )!;
      session.status = "stopped";
      return { session };
    },
    deleteSession: async (req) => {
      const idx = state.sessions.findIndex((s) => s.id === req.sessionId);
      if (idx >= 0) state.sessions.splice(idx, 1);
      return { sessionId: req.sessionId };
    },
    renameSession: async (req) => {
      const idx = state.sessions.findIndex((s) => s.id === req.sessionId);
      const current = state.sessions[idx]!;
      const session = { ...current, title: req.title };
      state.sessions[idx] = session;
      return { session };
    },
  } satisfies TestHandlers;
}
