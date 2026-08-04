import { createContext, useContext, useEffect, useMemo, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { WorkflowRuntime } from "@ora/workflow-runtime";
import { createMemoryWorkflowRuntime } from "@ora/workflow-runtime/memory";

const WorkflowRuntimeContext = createContext<WorkflowRuntime | null>(null);

interface WorkflowRuntimeProviderProps {
  children: ReactNode;
  /** Injected for tests; defaults to a process-lifetime memory runtime. */
  runtime?: WorkflowRuntime;
}

/**
 * Provides Host/Run repositories to the shell.
 * Default memory runtime is created once per provider mount (not on locale
 * change) so switching language cannot wipe mounts / in-flight runs.
 * Mock HITL schema copy uses the locale at first creation; HTTP backends will
 * carry localized copy on events instead.
 */
export function WorkflowRuntimeProvider({
  children,
  runtime: runtimeOverride,
}: WorkflowRuntimeProviderProps) {
  const { i18n } = useTranslation();
  const runtime = useMemo(() => {
    if (runtimeOverride !== undefined) {
      return runtimeOverride;
    }
    const locale = i18n.resolvedLanguage === "en-US" ? "en-US" as const : "zh-CN" as const;
    return createMemoryWorkflowRuntime({ locale });
    // Process-lifetime store: locale must not recreate Maps / engines.
    // eslint-disable-next-line react-hooks/exhaustive-deps -- intentional
  }, [runtimeOverride]);
  useEffect(() => {
    if (runtimeOverride !== undefined) {
      return;
    }
    return () => runtime.dispose();
  }, [runtime, runtimeOverride]);
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
