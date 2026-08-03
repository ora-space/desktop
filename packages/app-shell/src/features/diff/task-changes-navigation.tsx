import { createContext, useContext, type ReactNode } from "react";

interface TaskChangesNavigation {
  openFile: (path: string) => void;
}

const TaskChangesNavigationContext = createContext<TaskChangesNavigation | null>(null);

interface TaskChangesNavigationProviderProps {
  children: ReactNode;
  onOpenFile: (path: string) => void;
}

/** Shares the right-side Changes navigation action with nested conversation content. */
export function TaskChangesNavigationProvider({
  children,
  onOpenFile,
}: TaskChangesNavigationProviderProps) {
  return (
    <TaskChangesNavigationContext.Provider value={{ openFile: onOpenFile }}>
      {children}
    </TaskChangesNavigationContext.Provider>
  );
}

/** Returns the nearest task Changes navigator when the conversation belongs to a task. */
export function useTaskChangesNavigation(): TaskChangesNavigation | null {
  return useContext(TaskChangesNavigationContext);
}
