import { create } from "zustand";

/** The core-profile OpenSpec workflow commands the stepper walks through. */
export type WorkflowNodeId =
  "explore" | "propose" | "apply" | "sync" | "archive";

/** Lifecycle of one stepper node. Transitions are user-driven (suggest-only). */
export type WorkflowNodeStatus = "pending" | "running" | "done" | "skipped";

export interface WorkflowNode {
  id: WorkflowNodeId;
  status: WorkflowNodeStatus;
}

/** Node order shown in the stepper (OpenSpec `spec-driven` core profile). */
export const WORKFLOW_NODE_IDS: readonly WorkflowNodeId[] = [
  "explore",
  "propose",
  "apply",
  "sync",
  "archive",
] as const;

/** Side-steps rather than the propose → apply → archive spine; safe to skip. */
export const OPTIONAL_WORKFLOW_NODES: ReadonlySet<WorkflowNodeId> = new Set([
  "explore",
  "sync",
]);

/**
 * Light reminders prepended to the outgoing prompt (as hidden `agentText`) so the
 * agent runs the matching OpenSpec skill from the active Workspace's `.opencode/skills`.
 * Intentionally minimal — the skill itself owns change naming, file paths, and the
 * detailed per-artifact instructions, so the frontend only points at which skill.
 */
/** Skill directory name (under `.opencode/skills`) for each workflow node. */
export const WORKFLOW_SKILL: Record<WorkflowNodeId, string> = {
  explore: "openspec-explore",
  propose: "openspec-propose",
  apply: "openspec-apply-change",
  sync: "openspec-sync-specs",
  archive: "openspec-archive-change",
};

const REMINDER_BODY: Record<WorkflowNodeId, string> = {
  explore: "和我一起梳理这个需求。此阶段只探讨、不写代码。",
  propose:
    "为这个需求起草变更提案（proposal、specs、design、tasks）。产出后先暂停，等我 review 再进入实现。",
  apply: "实现已批准变更中的 tasks。",
  sync: "把主 specs 与本次变更同步。",
  archive: "归档这个已完成的变更。",
};

/**
 * Builds the Chinese reminder that points the agent at the OpenSpec skill by its
 * absolute path. `skillsDir` is the project's `.opencode/skills` — passed in
 * because the agent's isolated Workspace cwd may not contain it, so a resolved
 * Workspace path keeps the skill findable.
 *
 * That absolute path is the one Workspace anchor we hand the agent, so we must
 * fence it explicitly: it is for *reading* the skill only. All OpenSpec artifacts
 * (proposal / specs / design / tasks / changes) and the code implementation stay
 * in the agent's current working directory (its worktree) — otherwise the agent
 * follows the absolute path and writes them into the main Workspace.
 */
export function buildWorkflowReminder(
  nodeId: WorkflowNodeId,
  skillsDir: string,
): string {
  return `请使用位于 ${skillsDir} 的 ${WORKFLOW_SKILL[nodeId]} skill（该绝对路径仅用于读取 skill 说明）。所有 openspec 产物（proposal、specs、design、tasks、changes）以及代码实现都必须在你当前的工作目录中完成，不要写入 skill 所在的项目根目录。${REMINDER_BODY[nodeId]}`;
}

/** One artifact's completion state as reported by `openspec status --json`. */
export interface OpenSpecArtifact {
  id: string;
  status: string;
}

/** Parsed `openspec status --json` payload — best-effort, may be absent (see M4). */
export interface OpenSpecStatus {
  changeName?: string;
  artifacts: OpenSpecArtifact[];
  isComplete: boolean;
}

/** One session's spec-driven workflow state. Isolated per session key. */
export interface WorkflowRun {
  /** A workflow has been started for this session (survives hiding; cleared only by cancel). */
  active: boolean;
  /** Whether the stepper is currently shown. Toggling the composer button flips this. */
  visible: boolean;
  nodes: WorkflowNode[];
  currentNodeId: WorkflowNodeId | null;
  detected: OpenSpecStatus | null;
}

const freshNodes = (): WorkflowNode[] =>
  WORKFLOW_NODE_IDS.map((id) => ({ id, status: "pending" }));

/** Shared inactive run so selectors for sessions without a workflow stay stable. */
export const EMPTY_RUN: WorkflowRun = {
  active: false,
  visible: false,
  nodes: freshNodes(),
  currentNodeId: null,
  detected: null,
};

const withStatus = (
  nodes: WorkflowNode[],
  id: WorkflowNodeId,
  status: WorkflowNodeStatus,
): WorkflowNode[] =>
  nodes.map((node) => (node.id === id ? { ...node, status } : node));

interface WorkflowState {
  /** Per-session workflow runs, keyed by session id (or a pre-session task key). */
  runs: Record<string, WorkflowRun>;
  /**
   * Shows or hides one session's stepper. Starting fresh (no run yet) creates one;
   * afterwards this only flips visibility, so a hidden run keeps its progress until
   * explicitly cancelled with `reset`.
   */
  toggleVisible: (key: string) => void;
  launchNode: (key: string, id: WorkflowNodeId) => void;
  completeNode: (key: string, id: WorkflowNodeId) => void;
  skipNode: (key: string, id: WorkflowNodeId) => void;
  /** Records best-effort OpenSpec status parsed from that session's stream. */
  setDetected: (key: string, status: OpenSpecStatus) => void;
  reset: (key: string) => void;
  /** Moves a run to a new key (used when an optimistic session gets its real id). */
  rekey: (fromKey: string, toKey: string) => void;
}

/**
 * Frontend-only owner of the spec-driven workflow overlay, isolated per session so
 * switching sessions shows each one's own progress. It holds stepper state and the
 * pending prompt reminder; sending messages stays with the chat store, so this
 * store never touches the agent session directly.
 */
export const useWorkflowStore = create<WorkflowState>((set) => {
  const patch = (key: string, updater: (run: WorkflowRun) => WorkflowRun) =>
    set((state) => {
      const current = state.runs[key];
      if (current === undefined) return state;
      return { runs: { ...state.runs, [key]: updater(current) } };
    });

  return {
    runs: {},
    toggleVisible: (key) =>
      set((state) => {
        const run = state.runs[key];
        if (run !== undefined) {
          return {
            runs: { ...state.runs, [key]: { ...run, visible: !run.visible } },
          };
        }
        return {
          runs: {
            ...state.runs,
            [key]: {
              active: true,
              visible: true,
              nodes: freshNodes(),
              currentNodeId: null,
              detected: null,
            },
          },
        };
      }),
    launchNode: (key, id) =>
      patch(key, (run) => ({
        ...run,
        currentNodeId: id,
        nodes: withStatus(run.nodes, id, "running"),
      })),
    completeNode: (key, id) =>
      patch(key, (run) => ({
        ...run,
        currentNodeId: run.currentNodeId === id ? null : run.currentNodeId,
        nodes: withStatus(run.nodes, id, "done"),
      })),
    skipNode: (key, id) =>
      patch(key, (run) => ({
        ...run,
        currentNodeId: run.currentNodeId === id ? null : run.currentNodeId,
        nodes: withStatus(run.nodes, id, "skipped"),
      })),
    setDetected: (key, detected) => patch(key, (run) => ({ ...run, detected })),
    reset: (key) =>
      set((state) => {
        const next = { ...state.runs };
        delete next[key];
        return { runs: next };
      }),
    rekey: (fromKey, toKey) =>
      set((state) => {
        const run = state.runs[fromKey];
        if (run === undefined || fromKey === toKey) return state;
        const next = { ...state.runs };
        delete next[fromKey];
        next[toKey] = run;
        return { runs: next };
      }),
  };
});

/** One session's run, or a stable inactive placeholder when it has none. */
export function getRun(
  state: { runs: Record<string, WorkflowRun> },
  key: string,
): WorkflowRun {
  return state.runs[key] ?? EMPTY_RUN;
}

/** The first still-pending node — the stepper's suggested next step (starts at explore). */
export function suggestedNextNode(
  nodes: WorkflowNode[],
): WorkflowNodeId | null {
  return nodes.find((node) => node.status === "pending")?.id ?? null;
}

/**
 * The node a send should kick off — the highlighted next stage — or null when the
 * workflow is inactive/hidden or a stage is already running (so within-stage
 * messages are plain chat). Derived from state, so it never goes stale after a skip.
 */
export function kickNode(run: WorkflowRun): WorkflowNodeId | null {
  if (!run.active || !run.visible) return null;
  if (run.nodes.some((node) => node.status === "running")) return null;
  return suggestedNextNode(run.nodes);
}
