import { type ReactNode } from "react";
import { TaskChangesNavigationContext } from "./task-changes-navigation-context";

interface TaskChangesNavigationProviderProps {
  children: ReactNode;
  onOpenDiff: (path: string, line?: number) => void;
  onOpenWorkspaceFile: (path: string, line?: number, column?: number) => void;
}

/** Shares Diff and Files navigation actions with nested conversation content. */
export function TaskChangesNavigationProvider({
  children,
  onOpenDiff,
  onOpenWorkspaceFile,
}: TaskChangesNavigationProviderProps) {
  return (
    <TaskChangesNavigationContext.Provider
      value={{ openDiff: onOpenDiff, openWorkspaceFile: onOpenWorkspaceFile }}
    >
      {children}
    </TaskChangesNavigationContext.Provider>
  );
}
