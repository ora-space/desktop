import { useRef, useState, type ReactNode } from "react";
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
  IconFiles,
  IconGitBranch,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { usePlatform } from "@ora/platform";
import { TaskDiffView, type TaskDiffViewType } from "./task-diff-view";

const EXPANDED_PANEL_EXIT_MS = 180;

interface TaskChangesLayoutProps {
  taskId?: string;
  children: ReactNode;
}

/** Adds the independently resizable right-side review surface around existing workspace content. */
export function TaskChangesLayout({ taskId, children }: TaskChangesLayoutProps) {
  const { t } = useTranslation();
  const { windowControls } = usePlatform();
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [closing, setClosing] = useState(false);
  const [viewType, setViewType] = useState<TaskDiffViewType>("unified");
  const [fileTreeOpen, setFileTreeOpen] = useState(true);
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

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
        <TaskDiffView
          taskId={taskId}
          viewType={viewType}
          fileTreeOpen={fileTreeOpen}
          onFileTreeOpenChange={setFileTreeOpen}
        />
      </ResizablePanel>
    </ResizablePanelGroup>
  );

  return (
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
            className={`ora-changes-backdrop fixed inset-0 z-40 bg-background/45 backdrop-blur-[1.5px] ${closing ? "is-closing" : ""}`}
            onClick={toggleExpanded}
          />
          <section
            aria-label={t("diff.expandedPanel")}
            className={`ora-changes-overlay fixed bottom-2 left-[clamp(180px,12vw,260px)] right-2 top-2 z-50 overflow-hidden rounded-xl border border-border/80 bg-background shadow-[0_24px_90px_rgba(0,0,0,0.32),0_2px_12px_rgba(0,0,0,0.16)] ring-1 ring-foreground/5 dark:shadow-[0_28px_100px_rgba(0,0,0,0.62),0_2px_16px_rgba(0,0,0,0.32)] ${closing ? "is-closing" : ""}`}
          >
            <TaskDiffView
              taskId={taskId}
              viewType={viewType}
              fileTreeOpen={fileTreeOpen}
              expanded
              onFileTreeOpenChange={setFileTreeOpen}
            />
          </section>
        </>
      )}
      {taskId !== undefined && (
        <div
          role="group"
          aria-label={t("diff.changes")}
          className={`${expanded ? "fixed z-[60]" : "absolute z-30"} ${windowControls.kind === "overlay" ? "right-28" : "right-3"} top-3 flex h-8 items-center gap-0.5 rounded-lg border border-border/70 bg-background/95 p-0.5 shadow-sm backdrop-blur`}
        >
          {open && (
            <>
              {expanded && (
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
              <Button
                size="icon-sm"
                variant={fileTreeOpen ? "secondary" : "ghost"}
                className="size-7"
                aria-label={t("diff.toggleFileTree")}
                onClick={() => setFileTreeOpen((value) => !value)}
              >
                <IconFiles />
              </Button>
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
            variant={open ? "secondary" : "ghost"}
            className="h-7 px-2.5 shadow-none"
            aria-pressed={open}
            onClick={() => open ? close() : setOpen(true)}
          >
            <IconGitBranch />
            {t("diff.changes")}
          </Button>
        </div>
      )}
    </div>
  );
}
