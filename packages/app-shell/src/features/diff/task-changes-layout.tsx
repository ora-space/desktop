import { useCallback, useRef, useState, type ReactNode } from "react";
import {
  Button,
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
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
} from "./task-diff-view";
import { TaskChangesNavigationProvider } from "./task-changes-navigation";
import { WorkspaceFilesView } from "../files/workspace-files-view";

const EXPANDED_PANEL_EXIT_MS = 180;

interface TaskChangesLayoutProps {
  taskId?: string;
  children: ReactNode;
}

/** Adds the independently resizable right-side review surface around existing workspace content. */
export function TaskChangesLayout({ taskId, children }: TaskChangesLayoutProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [closing, setClosing] = useState(false);
  const [viewType, setViewType] = useState<TaskDiffViewType>("unified");
  const [fileTreeOpen, setFileTreeOpen] = useState(true);
  const [panel, setPanel] = useState<"changes" | "files">("changes");
  const [fileRequest, setFileRequest] = useState<TaskDiffFileRequest | undefined>();
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const fileRequestSequence = useRef(0);

  const openFile = useCallback((path: string) => {
    if (taskId === undefined) return;
    fileRequestSequence.current += 1;
    setFileRequest({ path, requestId: fileRequestSequence.current });
    setPanel("changes");
    setOpen(true);
  }, [taskId]);

  const close = () => {
    if (closeTimer.current !== null) clearTimeout(closeTimer.current);
    closeTimer.current = null;
    setOpen(false);
    setExpanded(false);
    setClosing(false);
    setViewType("unified");
  };

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

  const changesControls = (
    <div
      role="group"
      aria-label={t("diff.changes")}
      className="flex h-8 shrink-0 items-center gap-0.5 rounded-lg border border-border/70 bg-background/95 p-0.5 shadow-sm backdrop-blur"
    >
      {open && (
        <>
          {panel === "changes" && expanded && (
            <Button
              size="icon-sm"
              variant={viewType === "split" ? "secondary" : "ghost"}
              className="size-7"
              aria-label={t(viewType === "split" ? "diff.useUnifiedView" : "diff.useSplitView")}
              onClick={() => setViewType((value) => value === "unified" ? "split" : "unified")}
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
      <Button
        size="sm"
        variant={open && panel === "changes" ? "secondary" : "ghost"}
        className="h-7 px-2.5 shadow-none"
        aria-pressed={open && panel === "changes"}
        onClick={() => {
          if (open && panel === "changes") close();
          else {
            setPanel("changes");
            setOpen(true);
          }
        }}
      >
        <IconGitBranch />
        {t("diff.changes")}
      </Button>
      <Button
        size="sm"
        variant={open && panel === "files" ? "secondary" : "ghost"}
        className="h-7 px-2.5 shadow-none"
        aria-pressed={open && panel === "files"}
        onClick={() => {
          if (open && panel === "files") close();
          else {
            setPanel("files");
            setOpen(true);
          }
        }}
      >
        <IconFolderOpen />
        {t("files.files")}
      </Button>
    </div>
  );

  const panelContent = panel === "changes" ? (
    <TaskDiffView
      taskId={taskId!}
      viewType={viewType}
      fileTreeOpen={fileTreeOpen}
      fileRequest={fileRequest}
      toolbar={changesControls}
      onFileTreeOpenChange={setFileTreeOpen}
    />
  ) : (
    <WorkspaceFilesView key={taskId} taskId={taskId!} toolbar={changesControls} />
  );

  const workspaceContent = taskId === undefined || !open || expanded ? children : (
    <ResizablePanelGroup orientation="horizontal" className="min-h-0 min-w-0 flex-1">
      <ResizablePanel id="task-conversation" minSize={360}>
        {children}
      </ResizablePanel>
      <ResizableHandle
        withHandle
        aria-label={t("diff.resizePanel")}
        title={t("diff.resizePanel")}
        className="z-10 transition-colors hover:bg-ring focus-visible:bg-ring"
      />
      <ResizablePanel
        id="task-changes"
        className="ora-changes-side-panel"
        defaultSize={900}
        minSize={620}
        maxSize={1300}
        collapsible
        collapsedSize={0}
        groupResizeBehavior="preserve-pixel-size"
        onResize={(size) => {
          if (size.inPixels === 0) close();
        }}
      >
        {panelContent}
      </ResizablePanel>
    </ResizablePanelGroup>
  );

  return (
    <TaskChangesNavigationProvider onOpenFile={openFile}>
      <div className="relative flex min-h-0 min-w-0 flex-1">
        {taskId !== undefined && !open && (
          <div className="absolute right-4 top-2 z-30">{changesControls}</div>
        )}
        <div className="relative flex min-h-0 min-w-0 flex-1">
          <div
            className="flex min-h-0 min-w-0 flex-1"
            aria-hidden={expanded || undefined}
            inert={expanded || undefined}
          >
            {workspaceContent}
          </div>
          {taskId !== undefined && open && expanded && (
            <>
              <button
                type="button"
                aria-label={t("diff.closeExpandedPanel")}
                className={`ora-changes-backdrop absolute inset-0 z-40 bg-background/45 backdrop-blur-[1.5px] ${closing ? "is-closing" : ""}`}
                onClick={toggleExpanded}
              />
              <section
                aria-label={t("diff.expandedPanel")}
                className={`ora-changes-overlay absolute inset-2 z-50 overflow-hidden rounded-xl border border-border/80 bg-background shadow-[0_24px_90px_rgba(0,0,0,0.32),0_2px_12px_rgba(0,0,0,0.16)] ring-1 ring-foreground/5 dark:shadow-[0_28px_100px_rgba(0,0,0,0.62),0_2px_16px_rgba(0,0,0,0.32)] ${closing ? "is-closing" : ""}`}
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
