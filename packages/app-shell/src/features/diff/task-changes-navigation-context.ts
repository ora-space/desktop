import { createContext, useContext } from "react";

export interface TaskChangesNavigation {
  openDiff: (path: string, line?: number) => void;
  openWorkspaceFile: (path: string, line?: number, column?: number) => void;
  openWorkspaceDirectory?: (path: string) => void;
  openWorkspaceArtifact?: (
    path: string,
    line?: number,
    column?: number,
  ) => void;
}

export const TaskChangesNavigationContext =
  createContext<TaskChangesNavigation | null>(null);

/** Returns the nearest task Changes navigator when the conversation belongs to a task. */
export function useTaskChangesNavigation(): TaskChangesNavigation | null {
  return useContext(TaskChangesNavigationContext);
}
