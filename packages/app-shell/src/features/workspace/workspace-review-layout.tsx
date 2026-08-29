import {
  useCallback,
  useEffect,
  useLayoutEffect,
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
import type { FileNavigationLocation } from "../diff/task-changes-navigation-context";
import { WorkspaceReviewFilesPanel } from "../files/workspace-review-files-panel";
import type {
  WorkspaceDirectoryRequest,
  WorkspaceArtifactRequest,
  WorkspaceFileRequest,
} from "../files/workspace-files-view";
import { SurfaceHost } from "../surface/surface-host";
import { usePlatform } from "../../platform";
import { useSurfaceStore } from "../../state/stores/surface-store";
import {
  animatePanelWidth,
  cancelPanelWidthAnimation,
} from "../../lib/panel-motion";
import { usePersistHydrated } from "../../state/hooks/use-persist-hydrated";
import {
  buildReviewFilePersist,
  reviewContextKey,
  useReviewStore,
} from "../../state/stores/review-store";
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

// `workspaceId` gates the Changes surface: it is the workspace-diff API's identity for either
// an isolated task worktree or a project's main checkout, so a producer that has not resolved
// one yet (or chooses not to) simply gets a Changes-less review, same as `kind: "none"` today.
export type WorkspaceReviewContext =
  | { kind: "none" }
  | { kind: "project"; projectId: string; workspaceId?: string }
  | { kind: "task"; taskId: string; projectId: string; workspaceId?: string };

interface WorkspaceReviewLayoutProps {
  context: WorkspaceReviewContext;
  children: ReactNode;
  /** Fires when the side/expanded review panel opens or closes (not on expand-only). */
  onOpenChange?: (open: boolean) => void;
  /** Keeps stateful workspace children mounted while the review panel opens. */
  preserveWorkspaceOnReviewOpen?: boolean;
}

type ReviewPanel = "changes" | "files" | "surface";

/** Hosts every workspace review surface while preserving Ora's established panel interaction. */
export function WorkspaceReviewLayout({
  context,
  children,
  onOpenChange,
  preserveWorkspaceOnReviewOpen = false,
}: WorkspaceReviewLayoutProps) {
  const { t } = useTranslation();
  const { surfaces } = usePlatform();
  const sidePanelInstance = useSurfaceStore((s) => s.sidePanelInstance);
  // A surface already occupying the slot (e.g. the view remounted) opens at once.
  const [open, setOpen] = useState(sidePanelInstance !== null);
  const [expanded, setExpanded] = useState(false);
  const [closing, setClosing] = useState(false);
  const [viewType, setViewType] = useState<TaskDiffViewType>("unified");
  const [fileTreeOpen, setFileTreeOpen] = useState(true);
  const [panel, setPanel] = useState<ReviewPanel>(
    sidePanelInstance === null ? "files" : "surface",
  );
  const [fileRequest, setFileRequest] = useState<
    TaskDiffFileRequest | undefined
  >();
  const [workspaceFileRequest, setWorkspaceFileRequest] = useState<
    WorkspaceFileRequest | undefined
  >();
  const [reviewFilePath, setReviewFilePath] = useState<string | undefined>();
  const [workspaceDirectoryRequest, setWorkspaceDirectoryRequest] = useState<
    WorkspaceDirectoryRequest | undefined
  >();
  const [workspaceArtifactRequest, setWorkspaceArtifactRequest] = useState<
    WorkspaceArtifactRequest | undefined
  >();
  const [previousContextKind, setPreviousContextKind] = useState(context.kind);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const fileRequestSequence = useRef(0);
  const workspaceFileRequestSequence = useRef(0);
  const workspaceDirectoryRequestSequence = useRef(0);
  const workspaceArtifactRequestSequence = useRef(0);
  const onOpenChangeRef = useRef(onOpenChange);
  /** Mirrors `panel`/`open` for the surface-store subscription, which runs outside render. */
  const panelStateRef = useRef({ panel, open });
  /** Set while this layout itself releases the slot, so the subscription ignores its own write. */
  const releasingSurfaceRef = useRef(false);
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
  const workspaceId = context.kind === "none" ? undefined : context.workspaceId;
  const contextKey = reviewContextKey(context) ?? "none";
  const [previousContextKey, setPreviousContextKey] = useState(contextKey);
  const [restoredForContextKey, setRestoredForContextKey] = useState<
    string | null
  >(null);
  const reviewHydrated = usePersistHydrated(useReviewStore.persist);
  const reviewHydratedRef = useRef(reviewHydrated);

  useEffect(() => {
    reviewHydratedRef.current = reviewHydrated;
  }, [reviewHydrated]);

  // Keep the latest open-change listener for effect notifications.
  useEffect(() => {
    onOpenChangeRef.current = onOpenChange;
    panelStateRef.current = { panel, open };
  });

  const setReviewOpen = useCallback((next: boolean) => {
    setOpen((current) => (current === next ? current : next));
  }, []);

  const rememberReviewFile = useCallback((path: string) => {
    setReviewFilePath(path);
  }, []);

  const contextKind = context.kind;
  const applyStoredPreviewForPanel = useCallback(
    (panelToOpen: ReviewPanel) => {
      if (contextKind === "none") return;
      // A surface panel shows a native plugin webview, not a file; it has no
      // stored preview to replay.
      if (panelToOpen === "surface") return;
      const saved = useReviewStore.getState().byContext[contextKey];
      const savedFile = saved?.files[panelToOpen];
      if (savedFile === undefined) return;

      setReviewFilePath(savedFile.path);
      if (panelToOpen === "changes") {
        fileRequestSequence.current += 1;
        setFileRequest({
          path: savedFile.path,
          requestId: fileRequestSequence.current,
          line: savedFile.line,
          endLine: savedFile.endLine,
          side: savedFile.side,
        });
      } else {
        workspaceFileRequestSequence.current += 1;
        setWorkspaceFileRequest({
          path: savedFile.path,
          requestId: workspaceFileRequestSequence.current,
          line: savedFile.line,
          column: savedFile.column,
          endLine: savedFile.endLine,
        });
      }
    },
    [contextKey, contextKind],
  );

  const persistReviewLayout = useCallback(() => {
    if (contextKind === "none" || !reviewHydratedRef.current) return;
    if (restoredForContextKey !== contextKey) return;
    // A surface panel cannot be restored from disk (its native instance dies
    // with the process), so the snapshot keeps the last persistable panel.
    if (panel === "surface") return;
    const file = buildReviewFilePersist({
      open,
      panel,
      reviewFilePath,
      fileRequest,
      workspaceFileRequest,
    });
    useReviewStore.getState().upsertContext(contextKey, {
      open,
      panel,
      width: panelWidthRef.current,
      // Store under the live panel so the other tab keeps its own selection.
      ...(file !== undefined ? { files: { [panel]: file } } : {}),
    });
  }, [
    contextKey,
    contextKind,
    fileRequest,
    open,
    panel,
    restoredForContextKey,
    reviewFilePath,
    workspaceFileRequest,
  ]);

  useEffect(() => {
    persistReviewLayout();
  }, [persistReviewLayout]);

  /* eslint-disable react-hooks/set-state-in-effect -- apply persisted review snapshot before paint so persist cannot clobber disk with the previous context's open state */
  useLayoutEffect(() => {
    if (!reviewHydrated || contextKind === "none") return;
    // Restore is a one-shot per scope. Re-running would re-issue the stored file
    // request on every parent render and revert open/tab gestures the user made
    // after restore (layout effects observe the pre-commit store snapshot).
    if (restoredForContextKey === contextKey) return;

    setRestoredForContextKey(contextKey);

    const saved = useReviewStore.getState().byContext[contextKey];
    if (saved === undefined) return;

    if (!saved.open) {
      setReviewOpen(false);
      return;
    }

    // A context with no resolved workspace id has no Changes surface to restore into.
    const panelToOpen =
      saved.panel === "changes" && workspaceId === undefined
        ? "files"
        : saved.panel;

    panelWidthRef.current = saved.width;
    panelCurrentWidthRef.current = saved.width;
    panelWidthTouchedRef.current = true;

    setPanel(panelToOpen);
    setReviewOpen(true);
    applyStoredPreviewForPanel(panelToOpen);
  }, [
    applyStoredPreviewForPanel,
    contextKey,
    contextKind,
    restoredForContextKey,
    reviewHydrated,
    setReviewOpen,
    workspaceId,
  ]);
  /* eslint-enable react-hooks/set-state-in-effect */

  // Notify the parent after paint so we never setState on the parent during this
  // layout's render (React forbids updating WorkflowRunWorkspace from here).
  useEffect(() => {
    if (skipOpenNotifyRef.current) {
      skipOpenNotifyRef.current = false;
      return;
    }
    onOpenChangeRef.current?.(open);
  }, [open]);

  /**
   * Releases the embedded surface occupying the slot, if any. The store is
   * cleared synchronously so the `sidePanelInstance` effect never re-opens the
   * panel before the host confirms with a `closed` event.
   */
  const releaseSurface = useCallback(() => {
    const { sidePanelInstance: instance, setSidePanelInstance } =
      useSurfaceStore.getState();
    if (instance === null) return;
    releasingSurfaceRef.current = true;
    setSidePanelInstance(null);
    releasingSurfaceRef.current = false;
    void surfaces.close(instance).catch(() => undefined);
  }, [surfaces]);

  /** Tears the review surface down after its closing slide (or at once when already collapsed). */
  const finalizeClose = useCallback(() => {
    if (closeTimer.current !== null) clearTimeout(closeTimer.current);
    closeTimer.current = null;
    releaseSurface();
    setReviewOpen(false);
    setExpanded(false);
    setClosing(false);
    setViewType("unified");
  }, [releaseSurface, setReviewOpen]);

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
    // An embedded surface is context-independent, so it survives losing the task.
    if (context.kind === "none" && panel !== "surface") {
      // A pending slide aborts itself once the panel leaves the tree.
      setOpen(false);
      setExpanded(false);
      setClosing(false);
      setViewType("unified");
    }
  }
  if (contextKey !== previousContextKey) {
    setPreviousContextKey(contextKey);
    // Chat links and Files previews are checkout-scoped; clear them when the
    // selected project or task changes so paths from the previous root vanish.
    setFileRequest(undefined);
    setWorkspaceFileRequest(undefined);
    setReviewFilePath(undefined);
    setExpanded(false);
    setClosing(false);
    const savedForContext = useReviewStore.getState().byContext[contextKey];
    if (savedForContext !== undefined && !savedForContext.open) {
      setOpen(false);
    }
    setWorkspaceDirectoryRequest(undefined);
    setWorkspaceArtifactRequest(undefined);
    // A context with no resolved workspace id has no Changes surface; coerce so
    // the toolbar chrome matches the content that will actually render.
    if (workspaceId === undefined && panel === "changes") setPanel("files");
  }

  const openWorkspaceFile = useCallback(
    (path: string, location?: FileNavigationLocation) => {
      if (context.kind === "none") return;
      workspaceFileRequestSequence.current += 1;
      setWorkspaceFileRequest({
        path,
        requestId: workspaceFileRequestSequence.current,
        line: location?.line,
        column: location?.column,
        endLine: location?.endLine,
      });
      setReviewFilePath(path);
      setWorkspaceDirectoryRequest(undefined);
      setWorkspaceArtifactRequest(undefined);
      // Changes/Files take the slot over, so the embedded surface must go.
      releaseSurface();
      setPanel("files");
      setReviewOpen(true);
      if (panelAnimationRef.current !== null) slidePanelOpen();
    },
    [context.kind, releaseSurface, setReviewOpen, slidePanelOpen],
  );

  const openWorkspaceDirectory = useCallback(
    (path: string) => {
      if (context.kind === "none") return;
      workspaceDirectoryRequestSequence.current += 1;
      setWorkspaceDirectoryRequest({
        path,
        requestId: workspaceDirectoryRequestSequence.current,
      });
      setWorkspaceFileRequest(undefined);
      setWorkspaceArtifactRequest(undefined);
      // Changes/Files take the slot over, so the embedded surface must go.
      releaseSurface();
      setPanel("files");
      setReviewOpen(true);
      if (panelAnimationRef.current !== null) slidePanelOpen();
    },
    [context.kind, releaseSurface, setReviewOpen, slidePanelOpen],
  );

  const openWorkspaceArtifact = useCallback(
    (path: string, line?: number, column?: number) => {
      if (context.kind === "none") return;
      workspaceArtifactRequestSequence.current += 1;
      setWorkspaceArtifactRequest({
        path,
        requestId: workspaceArtifactRequestSequence.current,
        line,
        column,
      });
      setWorkspaceFileRequest(undefined);
      setWorkspaceDirectoryRequest(undefined);
      // Changes/Files take the slot over, so the embedded surface must go.
      releaseSurface();
      setPanel("files");
      setReviewOpen(true);
      if (panelAnimationRef.current !== null) slidePanelOpen();
    },
    [context.kind, releaseSurface, setReviewOpen, slidePanelOpen],
  );

  const openDiff = useCallback(
    (path: string, location?: FileNavigationLocation) => {
      // A context with no resolved workspace id has no Changes surface to open.
      if (workspaceId === undefined) {
        openWorkspaceFile(path, location);
        return;
      }
      fileRequestSequence.current += 1;
      setFileRequest({
        path,
        requestId: fileRequestSequence.current,
        line: location?.line,
        endLine: location?.endLine,
        side: location?.side,
      });
      setReviewFilePath(path);
      // Changes/Files take the slot over, so the embedded surface must go.
      releaseSurface();
      setPanel("changes");
      setReviewOpen(true);
      // A close slide may still be in flight; switch it back to opening.
      if (panelAnimationRef.current !== null) slidePanelOpen();
    },
    [
      openWorkspaceFile,
      releaseSurface,
      setReviewOpen,
      slidePanelOpen,
      workspaceId,
    ],
  );

  // The panel mounts collapsed, so opening (or re-opening after a context switch)
  // slides it out to the last settled width instead of snapping the conversation.
  useEffect(() => {
    if (!open) return;
    slidePanelOpen();
  }, [contextKey, open, slidePanelOpen]);

  // The surface store owns the right slot: claiming it shows the surface panel,
  // releasing it (host `closed` event, popout) collapses the panel again. The
  // store is subscribed directly so state changes happen in its callback.
  useEffect(
    () =>
      useSurfaceStore.subscribe((state, previous) => {
        if (releasingSurfaceRef.current) return;
        const instanceChanged =
          state.sidePanelInstance !== previous.sidePanelInstance;
        // Re-claiming the instance already in the slot must still win the panel
        // back (e.g. a file reference moved it to Files): the tick is what makes
        // that claim observable over no-op event traffic.
        const claimed =
          state.sidePanelClaimTick !== previous.sidePanelClaimTick;
        if (!instanceChanged && !claimed) return;
        if (state.sidePanelInstance !== null) {
          setPanel("surface");
          setReviewOpen(true);
          if (panelAnimationRef.current !== null) slidePanelOpen();
          return;
        }
        const current = panelStateRef.current;
        if (current.panel === "surface" && current.open) finalizeClose();
      }),
    [finalizeClose, setReviewOpen, slidePanelOpen],
  );

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
      const switching = panel !== next;
      if (switching) {
        setReviewFilePath(undefined);
        setFileRequest(undefined);
        setWorkspaceFileRequest(undefined);
      }
      // Changes/Files take the slot over, so the embedded surface must go.
      if (panel === "surface") releaseSurface();
      setPanel(next);
      setReviewOpen(true);
      applyStoredPreviewForPanel(next);
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
      {workspaceId !== undefined && (
        <PanelButton
          active={open && panel === "changes"}
          icon={<IconGitBranch />}
          label={t("diff.changes")}
          onClick={() => selectPanel("changes")}
        />
      )}
      {context.kind !== "none" && (
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
    panel === "surface" ? (
      sidePanelInstance === null ? null : (
        <SurfaceHost
          key={sidePanelInstance}
          instance={sidePanelInstance}
          toolbar={context.kind === "none" ? undefined : controls}
        />
      )
    ) : context.kind === "none" ? null : panel === "changes" &&
      workspaceId !== undefined ? (
      <TaskDiffView
        key={workspaceId}
        workspaceId={workspaceId}
        hasBaseline={context.kind === "task"}
        viewType={viewType}
        fileTreeOpen={fileTreeOpen}
        fileRequest={fileRequest}
        toolbar={controls}
        onFileTreeOpenChange={setFileTreeOpen}
        onFileNotFound={openWorkspaceFile}
        onPreviewPathChange={rememberReviewFile}
      />
    ) : (
      <WorkspaceReviewFilesPanel
        key={contextKey}
        projectId={context.projectId}
        taskId={context.kind === "task" ? context.taskId : undefined}
        toolbar={controls}
        fileRequest={workspaceFileRequest}
        onPreviewPathChange={rememberReviewFile}
        directoryRequest={workspaceDirectoryRequest}
        artifactRequest={workspaceArtifactRequest}
      />
    );

  // Keep the primary workspace under the same panel for the lifetime of a review
  // context. Switching from a bare child to this group would remount the workspace
  // when Changes opens and discard local UI state such as the workflow inspector.
  // The group also stays mounted while open (even under the expanded overlay) so
  // the review panel keeps its settled width; only its content yields to the
  // overlay to avoid mounting the same diff/file surface twice.
  const hasPanelHost = context.kind !== "none" || panel === "surface";
  const workspaceContent =
    !hasPanelHost || (!open && !preserveWorkspaceOnReviewOpen) ? (
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
          aria-hidden={!open || undefined}
          className={`z-10 transition-colors hover:bg-ring focus-visible:bg-ring ${open ? "" : "pointer-events-none invisible"}`}
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
            if (size.inPixels === 0 && open) close();
            else if (size.inPixels >= MIN_REVIEW_WIDTH) {
              panelWidthTouchedRef.current = true;
              panelWidthRef.current = size.inPixels;
              // Same gate as persistReviewLayout: before this scope has been
              // restored, upsertContext would seed a fresh entry from defaults
              // (open: false) for a panel the user currently has open.
              if (
                reviewHydratedRef.current &&
                restoredForContextKey === contextKey
              ) {
                useReviewStore.getState().upsertContext(contextKey, {
                  width: size.inPixels,
                });
              }
            }
          }}
        >
          {open && !expanded ? panelContent : null}
        </ResizablePanel>
      </ResizablePanelGroup>
    );

  return (
    <TaskChangesNavigationProvider
      onOpenDiff={openDiff}
      onOpenWorkspaceFile={openWorkspaceFile}
      onOpenWorkspaceDirectory={openWorkspaceDirectory}
      onOpenWorkspaceArtifact={openWorkspaceArtifact}
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
          {hasPanelHost && open && expanded && (
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
