import { createContext, useContext } from "react";

export interface TaskChangesNavigation {
  openFile: (path: string) => void;
}

export const TaskChangesNavigationContext = createContext<TaskChangesNavigation | null>(null);

/** Returns the nearest task Changes navigator when the conversation belongs to a task. */
export function useTaskChangesNavigation(): TaskChangesNavigation | null {
  return useContext(TaskChangesNavigationContext);
}
