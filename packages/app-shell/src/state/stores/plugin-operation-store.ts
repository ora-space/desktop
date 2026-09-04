import type { PluginInstallProgress } from "../../platform";
import { create } from "zustand";

export type PluginOperationKind =
  "install" | "update" | "activate" | "stop" | "uninstall";

export type PluginOperationActivity =
  | {
      state: "pending";
      kind: PluginOperationKind;
      progress: PluginInstallProgress | null;
    }
  | { state: "install_completed"; completionId: number };

interface PluginOperationState {
  activities: Record<string, PluginOperationActivity>;
  /** Starts an operation only when the same plugin has no conflicting work in flight. */
  begin: (pluginId: string, kind: PluginOperationKind) => boolean;
  /** Retains native package transfer progress across settings-page navigation. */
  reportTransferProgress: (progress: PluginInstallProgress) => void;
  /** Briefly records install success so the completion glyph can animate after query refresh. */
  completeInstall: (pluginId: string) => void;
  /** Clears the exact completion after its glyph animation ends. */
  consumeInstallCompletion: (pluginId: string, completionId: number) => void;
  /** Clears a settled operation so another lifecycle action can begin. */
  clear: (pluginId: string) => void;
}

let nextCompletionId = 1;

/** Owns plugin lifecycle activity independently of any settings row or card lifecycle. */
export const usePluginOperationStore = create<PluginOperationState>(
  (set, get) => ({
    activities: {},
    begin: (pluginId, kind) => {
      if (get().activities[pluginId]?.state === "pending") return false;
      set((state) => ({
        activities: {
          ...state.activities,
          [pluginId]: { state: "pending", kind, progress: null },
        },
      }));
      return true;
    },
    reportTransferProgress: (progress) => {
      const activity = get().activities[progress.pluginId];
      if (
        activity?.state !== "pending" ||
        (activity.kind !== "install" && activity.kind !== "update")
      )
        return;
      set((state) => ({
        activities: {
          ...state.activities,
          [progress.pluginId]: {
            state: "pending",
            kind: activity.kind,
            progress,
          },
        },
      }));
    },
    completeInstall: (pluginId) => {
      const completionId = nextCompletionId;
      nextCompletionId += 1;
      set((state) => ({
        activities: {
          ...state.activities,
          [pluginId]: { state: "install_completed", completionId },
        },
      }));
    },
    consumeInstallCompletion: (pluginId, completionId) => {
      const activity = get().activities[pluginId];
      if (
        activity?.state === "install_completed" &&
        activity.completionId === completionId
      ) {
        get().clear(pluginId);
      }
    },
    clear: (pluginId) => {
      set((state) => {
        const activities = { ...state.activities };
        delete activities[pluginId];
        return { activities };
      });
    },
  }),
);
