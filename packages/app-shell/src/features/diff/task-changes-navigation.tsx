import { type ReactNode } from "react";
import { TaskChangesNavigationContext } from "./task-changes-navigation-context";

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
