import { createContext, useContext, useMemo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { createMemoryWorkflowRuntime } from "./memory-workflow-runtime";
import type { WorkflowRuntime } from "./ports";

const WorkflowRuntimeContext = createContext<WorkflowRuntime | null>(null);

interface WorkflowRuntimeProviderProps {
  children: ReactNode;
  /** Injected for tests; defaults to a process-lifetime memory runtime. */
  runtime?: WorkflowRuntime;
}

/**
 * Provides Host/Run repositories to the shell.
 * One memory instance per provider mount so remounting tests get a clean slate
 * when they pass an explicit runtime.
 */
export function WorkflowRuntimeProvider({
  children,
  runtime: runtimeOverride,
}: WorkflowRuntimeProviderProps) {
  const { i18n } = useTranslation();
  const locale = i18n.resolvedLanguage === "en-US" ? "en-US" as const : "zh-CN" as const;
  const runtime = useMemo(
    () => runtimeOverride ?? createMemoryWorkflowRuntime({ locale }),
    [runtimeOverride, locale],
  );
  return (
    <WorkflowRuntimeContext.Provider value={runtime}>
      {children}
    </WorkflowRuntimeContext.Provider>
  );
}

/** Active workflow runtime (host mounts + graph runs). */
export function useWorkflowRuntime(): WorkflowRuntime {
  const runtime = useContext(WorkflowRuntimeContext);
  if (runtime === null) {
    throw new Error("useWorkflowRuntime requires WorkflowRuntimeProvider");
  }
  return runtime;
}
