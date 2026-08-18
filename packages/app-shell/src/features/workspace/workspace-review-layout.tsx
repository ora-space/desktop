import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import {
  Button,
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  type ResizablePanelHandle,
} from "@ora/ui";
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
import {
  TaskDiffView,
  type TaskDiffFileRequest,
  type TaskDiffViewType,
} from "../diff/task-diff-view";
import { TaskChangesNavigationProvider } from "../diff/task-changes-navigation";
import { WorkspaceReviewFilesPanel } from "../files/workspace-review-files-panel";
import type { WorkspaceFileRequest } from "../files/workspace-files-view";
import {
  animatePanelWidth,
  cancelPanelWidthAnimation,
} from "../../lib/panel-motion";
import {
  DEFAULT_REVIEW_WIDTH,
  MAX_REVIEW_WIDTH,
  MIN_REVIEW_WIDTH,
  responsiveReviewWidth,
} from "./workspace-review-layout-utils";
import "./workspace-review-layout.css";

const EXPANDED_PANEL_EXIT_MS = 180;
/** Matches the workflow editor's panel settle so the review slide feels identical. */
const REVIEW_PANEL_SLIDE_MS = 180;
const REVIEW_PANEL_COLLAPSE_THRESHOLD = MIN_REVIEW_WIDTH / 2;

export type WorkspaceReviewContext =
  | { kind: "none" }
  | { kind: "project"; projectId: string }
  | { kind: "task"; taskId: string; projectId: string };

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
  const [fileRequest, setFileRequest] = useState<
    TaskDiffFileRequest | undefined
  >();
  const [workspaceFileRequest, setWorkspaceFileRequest] = useState<
    WorkspaceFileRequest | undefined
  >();
  const [previousContextKind, setPreviousContextKind] = useState(context.kind);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const fileRequestSequence = useRef(0);
  const workspaceFileRequestSequence = useRef(0);
  const onOpenChangeRef = useRef(onOpenChange);
  const skipOpenNotifyRef = useRef(true);
  const panelRef = useRef<ResizablePanelHandle | null>(null);
  const panelAnimationRef = useRef<number | null>(null);
  /** Remembers the width the user last settled on, so reopen and restore reuse it. */
  const panelWidthRef = useRef(DEFAULT_REVIEW_WIDTH);
  /** Mirrors the live width (including transient drag values) for settle decisions. */
  const panelCurrentWidthRef = useRef(DEFAULT_REVIEW_WIDTH);
  /** Once the user drags (or a settle lands), stop re-adapting to the window. */
  const panelWidthTouchedRef = useRef(false);
  /** Host whose width drives the responsive opening width. */
  const contentRef = useRef<HTMLDivElement | null>(null);
  const taskId = context.kind === "task" ? context.taskId : undefined;
  const contextKey =
    context.kind === "none"
      ? "none"
      : context.kind === "project"
        ? `project:${context.projectId}`
        : `task:${context.taskId}`;
  const [previousContextKey, setPreviousContextKey] = useState(contextKey);

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

  /** Tears the review surface down after its closing slide (or at once when already collapsed). */
  const finalizeClose = useCallback(() => {
    if (closeTimer.current !== null) clearTimeout(closeTimer.current);
    closeTimer.current = null;
    setReviewOpen(false);
    setExpanded(false);
    setClosing(false);
    setViewType("unified");
  }, [setReviewOpen]);

  const close = useCallback(() => {
    cancelPanelWidthAnimation(panelAnimationRef);
    if (expanded) {
      // The overlay already covers the side panel, so a hidden slide adds nothing.
      finalizeClose();
      return;
    }
    animatePanelWidth({
      animationRef: panelAnimationRef,
      duration: REVIEW_PANEL_SLIDE_MS,
      panel: panelRef.current,
      targetWidth: 0,
      onComplete: finalizeClose,
    });
  }, [expanded, finalizeClose]);

  /** Slides the review panel out: the user's settled width, or a window-fit one until first use. */
  const slidePanelOpen = useCallback(() => {
    cancelPanelWidthAnimation(panelAnimationRef);
    const targetWidth = panelWidthTouchedRef.current
      ? panelWidthRef.current
      : responsiveReviewWidth(contentRef.current?.clientWidth ?? 0);
    animatePanelWidth({
      animationRef: panelAnimationRef,
      duration: REVIEW_PANEL_SLIDE_MS,
      panel: panelRef.current,
      targetWidth,
    });
  }, []);

  /** Snaps an undersized panel after release so direct dragging stays linear. */
  const settleReviewAfterResize = useCallback(() => {
    const width = panelCurrentWidthRef.current;
    if (width <= 0 || width >= MIN_REVIEW_WIDTH) return;
    animatePanelWidth({
      animationRef: panelAnimationRef,
      duration: REVIEW_PANEL_SLIDE_MS,
      panel: panelRef.current,
      targetWidth:
        width < REVIEW_PANEL_COLLAPSE_THRESHOLD ? 0 : MIN_REVIEW_WIDTH,
      onComplete: () => {
        if (width < REVIEW_PANEL_COLLAPSE_THRESHOLD) finalizeClose();
      },
    });
  }, [finalizeClose]);

  // React permits guarded render-time adjustment for state that is directly tied to
  // a prop. Closing here prevents one frame of stale review UI without an effect loop.
  // Parent notification happens via the `open` effect above — do not call onOpenChange here.
  if (context.kind !== previousContextKind) {
    setPreviousContextKind(context.kind);
    if (context.kind === "none") {
      // A pending slide aborts itself once the panel leaves the tree.
      setOpen(false);
      setExpanded(false);
      setClosing(false);
      setViewType("unified");
    }
  }
  if (contextKey !== previousContextKey) {
    setPreviousContextKey(contextKey);
    // Chat links and Files previews are task-scoped; keep them from opening a
    // path that only existed in the previous worktree.
    setFileRequest(undefined);
    setWorkspaceFileRequest(undefined);
  }

  const openDiff = useCallback(
    (path: string, line?: number) => {
      if (taskId === undefined) return;
      fileRequestSequence.current += 1;
      setFileRequest({
        path,
        requestId: fileRequestSequence.current,
        line,
      });
      setPanel("changes");
      setReviewOpen(true);
      // A close slide may still be in flight; switch it back to opening.
      if (panelAnimationRef.current !== null) slidePanelOpen();
    },
    [setReviewOpen, slidePanelOpen, taskId],
  );

  const openWorkspaceFile = useCallback(
    (path: string, line?: number, column?: number) => {
      if (taskId === undefined) return;
      workspaceFileRequestSequence.current += 1;
      setWorkspaceFileRequest({
        path,
        requestId: workspaceFileRequestSequence.current,
        line,
        column,
      });
      setPanel("files");
      setReviewOpen(true);
      if (panelAnimationRef.current !== null) slidePanelOpen();
    },
    [setReviewOpen, slidePanelOpen, taskId],
  );

  // The panel mounts collapsed, so opening (or re-opening after a context switch)
  // slides it out to the last settled width instead of snapping the conversation.
  useEffect(() => {
    if (!open) return;
    slidePanelOpen();
  }, [contextKey, open, slidePanelOpen]);

  // Before the user picks a width, keep the panel matched to the window: maximizing
  // opens it wider, restoring snaps it narrower, without ever fighting a drag.
  useEffect(() => {
    if (!open || typeof ResizeObserver === "undefined") return;
    const content = contentRef.current;
    if (content === null) return;
    let lastWidth = content.clientWidth;
    const observer = new ResizeObserver(() => {
      if (panelWidthTouchedRef.current) return;
      const nextWidth = content.clientWidth;
      if (Math.abs(nextWidth - lastWidth) < 8) return;
      lastWidth = nextWidth;
      cancelPanelWidthAnimation(panelAnimationRef);
      animatePanelWidth({
        animationRef: panelAnimationRef,
        duration: REVIEW_PANEL_SLIDE_MS,
        panel: panelRef.current,
        targetWidth: responsiveReviewWidth(nextWidth),
      });
    });
    observer.observe(content);
    return () => observer.disconnect();
  }, [open]);

  // Never let a pending slide write to a panel that already left the tree.
  useEffect(() => () => cancelPanelWidthAnimation(panelAnimationRef), []);

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
      // A close slide may still be in flight; switch it back to opening.
      if (panelAnimationRef.current !== null) slidePanelOpen();
    }
  };

  const controls = (
    <div
      role="group"
      aria-label={t("review.panels")}
      className="ora-diff-toolbar__view-group flex h-8 shrink-0 items-center gap-0.5 rounded-lg border border-border/70 bg-background/95 p-0.5 shadow-sm backdrop-blur"
    >
      {open && (
        <>
          {panel === "changes" && expanded && (
            <Button
              size="icon-sm"
              variant={viewType === "split" ? "secondary" : "ghost"}
              className="size-7"
              aria-label={t(
                viewType === "split"
                  ? "diff.useUnifiedView"
                  : "diff.useSplitView",
              )}
              onClick={() =>
                setViewType((value) =>
                  value === "unified" ? "split" : "unified",
                )
              }
            >
              <IconColumns2 />
            </Button>
          )}
          {panel === "changes" && (
            <Button
              size="icon-sm"
              variant={fileTreeOpen ? "secondary" : "ghost"}
              className="size-7"
              aria-label={t("diff.toggleFileTree")}
              onClick={() => setFileTreeOpen((value) => !value)}
            >
              {fileTreeOpen ? (
                <IconLayoutSidebarRightCollapse />
              ) : (
                <IconLayoutSidebarRightExpand />
              )}
            </Button>
          )}
          <Button
            size="icon-sm"
            variant={expanded ? "secondary" : "ghost"}
            className="size-7"
            aria-label={t(expanded ? "diff.restorePanel" : "diff.expandPanel")}
            onClick={toggleExpanded}
          >
            {expanded ? <IconArrowsMinimize /> : <IconArrowsMaximize />}
          </Button>
          <span className="mx-0.5 h-4 w-px bg-border/70" aria-hidden="true" />
        </>
      )}
      {context.kind === "task" && (
        <>
          <PanelButton
            active={open && panel === "changes"}
            icon={<IconGitBranch />}
            label={t("diff.changes")}
            onClick={() => selectPanel("changes")}
          />
          <PanelButton
            active={open && panel === "files"}
            icon={<IconFolderOpen />}
            label={t("files.files")}
            onClick={() => selectPanel("files")}
          />
        </>
      )}
      {context.kind === "project" && (
        <PanelButton
          active={open && panel === "files"}
          icon={<IconFolderOpen />}
          label={t("files.files")}
          onClick={() => selectPanel("files")}
        />
      )}
    </div>
  );

  const panelContent =
    context.kind === "none" ? null : panel === "changes" &&
      context.kind === "task" ? (
      <TaskDiffView
        key={context.taskId}
        taskId={context.taskId}
        viewType={viewType}
        fileTreeOpen={fileTreeOpen}
        fileRequest={fileRequest}
        toolbar={controls}
        onFileTreeOpenChange={setFileTreeOpen}
        onFileNotFound={openWorkspaceFile}
      />
    ) : (
      <WorkspaceReviewFilesPanel
        key={contextKey}
        projectId={context.projectId}
        taskId={context.kind === "task" ? context.taskId : undefined}
        toolbar={controls}
        fileRequest={workspaceFileRequest}
      />
    );

  // The group stays mounted while open (even under the expanded overlay) so the
  // review panel keeps its settled width; only its content yields to the overlay
  // to avoid mounting the same diff/file surface twice.
  const workspaceContent =
    context.kind === "none" || !open ? (
      children
    ) : (
      <ResizablePanelGroup
        orientation="horizontal"
        className="min-h-0 min-w-0 flex-1"
        onLayoutChanged={(_layout, meta) => {
          if (meta.isUserInteraction) settleReviewAfterResize();
        }}
      >
        <ResizablePanel id="workspace-primary" minSize={360}>
          {children}
        </ResizablePanel>
        <ResizableHandle
          withHandle
          aria-label={t("diff.resizePanel")}
          title={t("diff.resizePanel")}
          className="z-10 transition-colors hover:bg-ring focus-visible:bg-ring"
          onPointerDown={() => cancelPanelWidthAnimation(panelAnimationRef)}
        />
        <ResizablePanel
          id="workspace-review"
          panelRef={panelRef}
          className="ora-review-side-panel"
          defaultSize={0}
          // A pixel min would snap scripted slides onto it; the settle callback
          // restores the effective minimum after the user lets go (workflow pattern).
          minSize={1}
          maxSize={MAX_REVIEW_WIDTH}
          collapsible
          collapsedSize={0}
          groupResizeBehavior="preserve-pixel-size"
          onResize={(size) => {
            // Scripted slides report intermediate sizes; only settle on stable ones.
            if (panelAnimationRef.current !== null) return;
            panelCurrentWidthRef.current = size.inPixels;
            if (size.inPixels === 0) close();
            else if (size.inPixels >= MIN_REVIEW_WIDTH) {
              panelWidthTouchedRef.current = true;
              panelWidthRef.current = size.inPixels;
            }
          }}
        >
          {expanded ? null : panelContent}
        </ResizablePanel>
      </ResizablePanelGroup>
    );

  return (
    <TaskChangesNavigationProvider
      onOpenDiff={openDiff}
      onOpenWorkspaceFile={openWorkspaceFile}
    >
      <div className="relative flex min-h-0 min-w-0 flex-1">
        {context.kind !== "none" && !open && (
          <div className="absolute right-4 top-2 z-30">{controls}</div>
        )}
        <div className="relative flex min-h-0 min-w-0 flex-1">
          <div
            ref={contentRef}
            className="flex min-h-0 min-w-0 flex-1"
            aria-hidden={expanded || undefined}
            inert={expanded || undefined}
          >
            {workspaceContent}
          </div>
          {context.kind !== "none" && open && expanded && (
            <>
              <button
                type="button"
                aria-label={t("diff.closeExpandedPanel")}
                className={`ora-review-backdrop absolute inset-0 z-40 bg-background/45 backdrop-blur-[1.5px] ${closing ? "is-closing" : ""}`}
                onClick={toggleExpanded}
              />
              <section
                aria-label={t("diff.expandedPanel")}
                className={`ora-review-overlay absolute inset-2 z-50 overflow-hidden rounded-xl border border-border/80 bg-background shadow-[0_24px_90px_rgba(0,0,0,0.32),0_2px_12px_rgba(0,0,0,0.16)] ring-1 ring-foreground/5 dark:shadow-[0_28px_100px_rgba(0,0,0,0.62),0_2px_16px_rgba(0,0,0,0.32)] ${closing ? "is-closing" : ""}`}
              >
                {panelContent}
              </section>
            </>
          )}
        </div>
      </div>
    </TaskChangesNavigationProvider>
  );
}

function PanelButton({
  active,
  icon,
  label,
  onClick,
}: {
  active: boolean;
  icon: ReactNode;
  label: string;
  onClick: () => void;
}) {
  return (
    <Button
      size="sm"
      variant={active ? "secondary" : "ghost"}
      className="h-7 px-2.5 shadow-none"
      aria-pressed={active}
      onClick={onClick}
    >
      {icon}
      <span className="ora-diff-toolbar__view-label">{label}</span>
    </Button>
  );
}
