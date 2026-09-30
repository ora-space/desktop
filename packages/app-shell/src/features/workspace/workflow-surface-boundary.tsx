import { Component, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { Button } from "@ora/ui";

interface WorkflowSurfaceBoundaryProps {
  scopeId: string;
  children: ReactNode;
  onClose: () => void;
  recoveryRevision?: number;
  onRenderFailure?: () => void;
}

/** Keeps a workflow render failure inside its pane so workspace navigation remains usable. */
export function WorkflowSurfaceBoundary({
  scopeId,
  children,
  onClose,
  recoveryRevision = 0,
  onRenderFailure,
}: WorkflowSurfaceBoundaryProps) {
  const { t } = useTranslation();
  return (
    <WorkflowRenderBoundary
      key={scopeId}
      recoveryRevision={recoveryRevision}
      onRenderFailure={onRenderFailure}
      fallback={
        <main
          id="main-content"
          className="flex min-w-0 flex-1 items-center justify-center p-6"
        >
          <div role="alert" className="max-w-md space-y-3">
            <h2 className="text-sm font-semibold">
              {t("workspace.workflowRenderError.title")}
            </h2>
            <p className="text-sm text-muted-foreground">
              {t("workspace.workflowRenderError.description")}
            </p>
            <Button onClick={onClose}>
              {t("workspace.workflowRenderError.close")}
            </Button>
          </div>
        </main>
      }
    >
      {children}
    </WorkflowRenderBoundary>
  );
}

/** React's render-error lifecycle must be owned by a class; its scope is one workflow surface. */
class WorkflowRenderBoundary extends Component<
  {
    children: ReactNode;
    fallback: ReactNode;
    recoveryRevision: number;
    onRenderFailure?: () => void;
  },
  { failed: boolean; recoveryRevision: number }
> {
  state = { failed: false, recoveryRevision: this.props.recoveryRevision };

  /** Latches a failed render until its owner requests recovery or the pane is closed. */
  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true };
  }

  /** Retries only an explicit recovery, keeping healthy editor selection and flush lifecycles intact. */
  static getDerivedStateFromProps(
    props: { recoveryRevision: number },
    state: { recoveryRevision: number },
  ): { failed: boolean; recoveryRevision: number } | null {
    return props.recoveryRevision === state.recoveryRevision
      ? null
      : { failed: false, recoveryRevision: props.recoveryRevision };
  }

  /** Reports failure after React has unmounted the child and released its registered actions. */
  componentDidCatch(): void {
    this.props.onRenderFailure?.();
  }

  /** Renders the recovery control without retrying a deterministically broken child tree. */
  render() {
    return this.state.failed ? this.props.fallback : this.props.children;
  }
}
