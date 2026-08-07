import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Button, ResizableHandle, ResizablePanel, ResizablePanelGroup } from "@ora/ui";
import {
  IconArrowsMaximize,
  IconArrowsMinimize,
  IconColumns2,
  IconFolderOpen,
  IconGitBranch,
  IconLayoutSidebarRightCollapse,
  IconLayoutSidebarRightExpand,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { TaskDiffView, type TaskDiffFileRequest, type TaskDiffViewType } from "../diff/task-diff-view";
import { TaskChangesNavigationProvider } from "../diff/task-changes-navigation";
import { WorkspaceReviewFilesPanel } from "../files/workspace-review-files-panel";
import "./workspace-review-layout.css";

const EXPANDED_PANEL_EXIT_MS = 180;

export type WorkspaceReviewContext =
  | { kind: "none" }
  | { kind: "project"; projectId: string; projectRootPath: string }
  | { kind: "task"; taskId: string; projectId: string; projectRootPath: string };

interface WorkspaceReviewLayoutProps {
  context: WorkspaceReviewContext;
  children: ReactNode;
  /** Fires when the side/expanded review panel opens or closes (not on expand-only). */
  onOpenChange?: (open: boolean) => void;
}

type ReviewPanel = "changes" | "files";

/** Hosts every workspace review surface while preserving Ora's established panel interaction. */
export function WorkspaceReviewLayout({
  context,
  children,
  onOpenChange,
}: WorkspaceReviewLayoutProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [closing, setClosing] = useState(false);
  const [viewType, setViewType] = useState<TaskDiffViewType>("unified");
  const [fileTreeOpen, setFileTreeOpen] = useState(true);
  const [panel, setPanel] = useState<ReviewPanel>("files");
  const [fileRequest, setFileRequest] = useState<TaskDiffFileRequest | undefined>();
  const [previousContextKind, setPreviousContextKind] = useState(context.kind);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const fileRequestSequence = useRef(0);
  const onOpenChangeRef = useRef(onOpenChange);
  const skipOpenNotifyRef = useRef(true);
  const taskId = context.kind === "task" ? context.taskId : undefined;
  const contextKey = context.kind === "none" ? "none" : context.kind === "project" ? `project:${context.projectId}` : `task:${context.taskId}`;

  // Keep the latest open-change listener for effect notifications.
  useEffect(() => {
    onOpenChangeRef.current = onOpenChange;
  });

  const setReviewOpen = useCallback((next: boolean) => {
    setOpen((current) => (current === next ? current : next));
  }, []);

  // Notify the parent after paint so we never setState on the parent during this
  // layout's render (React forbids updating WorkflowRunWorkspace from here).
  useEffect(() => {
    if (skipOpenNotifyRef.current) {
      skipOpenNotifyRef.current = false;
      return;
    }
    onOpenChangeRef.current?.(open);
  }, [open]);

  const close = useCallback(() => {
    if (closeTimer.current !== null) clearTimeout(closeTimer.current);
    closeTimer.current = null;
    setReviewOpen(false);
    setExpanded(false);
    setClosing(false);
    setViewType("unified");
  }, [setReviewOpen]);

  // React permits guarded render-time adjustment for state that is directly tied to
  // a prop. Closing here prevents one frame of stale review UI without an effect loop.
  // Parent notification happens via the `open` effect above — do not call onOpenChange here.
  if (context.kind !== previousContextKind) {
    setPreviousContextKind(context.kind);
    if (context.kind === "none") {
      setOpen(false);
      setExpanded(false);
      setClosing(false);
      setViewType("unified");
    }
  }

  const openFile = useCallback((path: string) => {
    if (taskId === undefined) return;
    fileRequestSequence.current += 1;
    setFileRequest({ path, requestId: fileRequestSequence.current });
    setPanel("changes");
    setReviewOpen(true);
  }, [setReviewOpen, taskId]);

  const toggleExpanded = () => {
    if (!expanded) {
      setExpanded(true);
      return;
    }
    if (closing) return;
    setClosing(true);
    closeTimer.current = setTimeout(() => {
      closeTimer.current = null;
      setExpanded(false);
      setClosing(false);
      setViewType("unified");
    }, EXPANDED_PANEL_EXIT_MS);
  };

  const selectPanel = (next: ReviewPanel) => {
    if (open && panel === next) close();
    else {
      setPanel(next);
      setReviewOpen(true);
    }
  };

  const controls = (
    <div role="group" aria-label={t("review.panels")} className="ora-diff-toolbar__view-group flex h-8 shrink-0 items-center gap-0.5 rounded-lg border border-border/70 bg-background/95 p-0.5 shadow-sm backdrop-blur">
      {open && (
        <>
          {panel === "changes" && expanded && (
            <Button size="icon-sm" variant={viewType === "split" ? "secondary" : "ghost"} className="size-7" aria-label={t(viewType === "split" ? "diff.useUnifiedView" : "diff.useSplitView")} onClick={() => setViewType((value) => value === "unified" ? "split" : "unified")}><IconColumns2 /></Button>
          )}
          {panel === "changes" && (
            <Button size="icon-sm" variant={fileTreeOpen ? "secondary" : "ghost"} className="size-7" aria-label={t("diff.toggleFileTree")} onClick={() => setFileTreeOpen((value) => !value)}>
              {fileTreeOpen ? <IconLayoutSidebarRightCollapse /> : <IconLayoutSidebarRightExpand />}
            </Button>
          )}
          <Button size="icon-sm" variant={expanded ? "secondary" : "ghost"} className="size-7" aria-label={t(expanded ? "diff.restorePanel" : "diff.expandPanel")} onClick={toggleExpanded}>
            {expanded ? <IconArrowsMinimize /> : <IconArrowsMaximize />}
          </Button>
          <span className="mx-0.5 h-4 w-px bg-border/70" aria-hidden="true" />
        </>
      )}
      {context.kind === "task" && (
        <>
          <PanelButton active={open && panel === "changes"} icon={<IconGitBranch />} label={t("diff.changes")} onClick={() => selectPanel("changes")} />
          <PanelButton active={open && panel === "files"} icon={<IconFolderOpen />} label={t("files.files")} onClick={() => selectPanel("files")} />
        </>
      )}
      {context.kind === "project" && (
        <PanelButton active={open && panel === "files"} icon={<IconFolderOpen />} label={t("files.files")} onClick={() => selectPanel("files")} />
      )}
    </div>
  );

  const panelContent = context.kind === "none" ? null : panel === "changes" && context.kind === "task" ? (
    <TaskDiffView taskId={context.taskId} viewType={viewType} fileTreeOpen={fileTreeOpen} fileRequest={fileRequest} toolbar={controls} onFileTreeOpenChange={setFileTreeOpen} />
  ) : (
    <WorkspaceReviewFilesPanel
      key={contextKey}
      projectId={context.projectId}
      projectRootPath={context.projectRootPath}
      taskId={context.kind === "task" ? context.taskId : undefined}
      toolbar={controls}
    />
  );

  const workspaceContent = context.kind === "none" || !open || expanded ? children : (
    <ResizablePanelGroup orientation="horizontal" className="min-h-0 min-w-0 flex-1">
      <ResizablePanel id="workspace-primary" minSize={360}>{children}</ResizablePanel>
      <ResizableHandle withHandle aria-label={t("diff.resizePanel")} title={t("diff.resizePanel")} className="z-10 transition-colors hover:bg-ring focus-visible:bg-ring" />
      <ResizablePanel id="workspace-review" className="ora-review-side-panel" defaultSize={900} minSize={620} maxSize={1300} collapsible collapsedSize={0} groupResizeBehavior="preserve-pixel-size" onResize={(size) => { if (size.inPixels === 0) close(); }}>
        {panelContent}
      </ResizablePanel>
    </ResizablePanelGroup>
  );

  return (
    <TaskChangesNavigationProvider onOpenFile={openFile}>
      <div className="relative flex min-h-0 min-w-0 flex-1">
        {context.kind !== "none" && !open && <div className="absolute right-4 top-2 z-30">{controls}</div>}
        <div className="relative flex min-h-0 min-w-0 flex-1">
          <div className="flex min-h-0 min-w-0 flex-1" aria-hidden={expanded || undefined} inert={expanded || undefined}>{workspaceContent}</div>
          {context.kind !== "none" && open && expanded && (
            <>
              <button type="button" aria-label={t("diff.closeExpandedPanel")} className={`ora-review-backdrop absolute inset-0 z-40 bg-background/45 backdrop-blur-[1.5px] ${closing ? "is-closing" : ""}`} onClick={toggleExpanded} />
              <section aria-label={t("diff.expandedPanel")} className={`ora-review-overlay absolute inset-2 z-50 overflow-hidden rounded-xl border border-border/80 bg-background shadow-[0_24px_90px_rgba(0,0,0,0.32),0_2px_12px_rgba(0,0,0,0.16)] ring-1 ring-foreground/5 dark:shadow-[0_28px_100px_rgba(0,0,0,0.62),0_2px_16px_rgba(0,0,0,0.32)] ${closing ? "is-closing" : ""}`}>{panelContent}</section>
            </>
          )}
        </div>
      </div>
    </TaskChangesNavigationProvider>
  );
}

function PanelButton({ active, icon, label, onClick }: { active: boolean; icon: ReactNode; label: string; onClick: () => void }) {
  return (
    <Button size="sm" variant={active ? "secondary" : "ghost"} className="h-7 px-2.5 shadow-none" aria-pressed={active} onClick={onClick}>
      {icon}<span className="ora-diff-toolbar__view-label">{label}</span>
    </Button>
  );
}
