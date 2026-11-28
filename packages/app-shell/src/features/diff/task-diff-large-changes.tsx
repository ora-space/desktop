import { useEffect, useMemo, useRef, useState } from "react";
import type { WorkspaceDiffScope } from "@ora/contracts";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Button } from "@ora/ui";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";
import { pathsMatchForWorkspace } from "../../lib/workspace-path";
import {
  isWorkspaceDiffTooLarge,
  useWorkspaceDiff,
} from "../../state/hooks/use-workspace-diff";
import {
  fileNavigationLocation,
  type FileNavigationLocation,
} from "./task-changes-navigation-context";
import { parseTaskDiffPatch } from "./task-diff-data";
import { TaskDiffFocusBody } from "./task-diff-focus-view";
import { DiffLoadingState, DiffMessage } from "./task-diff-states";
import type { TaskDiffFileRequest, TaskDiffViewType } from "./task-diff-view";

const FILE_ROW_HEIGHT = 32;
const VIRTUALIZE_FILE_COUNT = 50;

interface TaskDiffLargeChangesProps {
  workspaceId: string;
  scope: WorkspaceDiffScope;
  viewType: TaskDiffViewType;
  fileRequest?: TaskDiffFileRequest;
  onOpenFile?: (path: string, location?: FileNavigationLocation) => void;
  onPreviewPathChange?: (path: string) => void;
}

/** Lets the user review one bounded file patch after the workspace patch exceeds its limit. */
export function TaskDiffLargeChanges({
  workspaceId,
  scope,
  viewType,
  fileRequest,
  onOpenFile,
  onPreviewPathChange,
}: TaskDiffLargeChangesProps) {
  const { t } = useTranslation();
  const overview = useWorkspaceDiff(workspaceId, scope, true, {
    kind: "files",
  });
  const files = overview.data?.files;
  const [selection, setSelection] = useState<{
    path: string;
    requestId: number | undefined;
  }>();
  const scrollRef = useRef<HTMLDivElement>(null);
  const missingRequest = useRef<number | undefined>(undefined);
  const requestedFile =
    fileRequest === undefined
      ? undefined
      : files?.find(
          (file) =>
            pathsMatchForWorkspace(fileRequest.path, file.path) ||
            (file.oldPath !== null &&
              pathsMatchForWorkspace(fileRequest.path, file.oldPath)),
        );
  const selectRequested = fileRequest?.requestId !== selection?.requestId;
  const activeFile =
    (selectRequested ? requestedFile : undefined) ??
    files?.find((file) => file.path === selection?.path) ??
    files?.[0];
  const preview = useWorkspaceDiff(
    workspaceId,
    scope,
    activeFile !== undefined,
    {
      kind: "file",
      path: activeFile?.path ?? "",
      oldPath: activeFile?.oldPath ?? null,
    },
    overview.data?.headCommitId,
  );
  const previewOutdated =
    preview.data !== undefined &&
    preview.data.headCommitId !== overview.data?.headCommitId;
  const patch = previewOutdated ? undefined : preview.data?.patch;
  const activePath = activeFile?.path;
  const originalPath = activeFile?.oldPath;
  const parsed = useMemo(
    () =>
      patch === undefined || activePath === undefined
        ? []
        : parseTaskDiffPatch(patch).map((file) => ({
            ...file,
            oldPath:
              file.type === "add" ? file.oldPath : (originalPath ?? activePath),
            newPath: file.type === "delete" ? file.newPath : activePath,
          })),
    [patch, activePath, originalPath],
  );
  const virtualized = (files?.length ?? 0) > VIRTUALIZE_FILE_COUNT;
  // eslint-disable-next-line react-hooks/incompatible-library -- the virtual window reads live scroll geometry during render
  const fileList = useVirtualizer({
    count: virtualized ? (files?.length ?? 0) : 0,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => FILE_ROW_HEIGHT,
    overscan: 8,
    initialRect: { width: 0, height: 320 },
  });

  useEffect(() => {
    if (activePath !== undefined) onPreviewPathChange?.(activePath);
  }, [activePath, onPreviewPathChange]);

  useEffect(() => {
    if (
      fileRequest === undefined ||
      !selectRequested ||
      files === undefined ||
      overview.isFetching ||
      overview.isError ||
      requestedFile !== undefined ||
      missingRequest.current === fileRequest.requestId
    ) {
      return;
    }
    missingRequest.current = fileRequest.requestId;
    onOpenFile?.(fileRequest.path, fileNavigationLocation(fileRequest));
  }, [
    fileRequest,
    files,
    onOpenFile,
    overview.isError,
    overview.isFetching,
    requestedFile,
    selectRequested,
  ]);

  useEffect(() => {
    if (requestedFile === undefined || files === undefined || !selectRequested)
      return;
    fileList.scrollToIndex(files.indexOf(requestedFile), { align: "auto" });
  }, [fileList, files, requestedFile, selectRequested, virtualized]);

  if (overview.isPending) return <DiffLoadingState />;
  if (files === undefined) {
    return (
      <DiffMessage
        title={t("diff.loadError")}
        detail={localizeContractError(overview.error, t)}
        action={
          <Button onClick={() => void overview.refetch()}>
            {t("diff.retry")}
          </Button>
        }
      />
    );
  }
  if (activeFile === undefined) {
    return (
      <DiffMessage
        title={t("diff.noChanges")}
        detail={t("diff.noChangesDetail")}
      />
    );
  }

  const target =
    activeFile === requestedFile && selectRequested ? fileRequest : undefined;
  const tooLarge = isWorkspaceDiffTooLarge(preview.error);
  const rows = virtualized
    ? fileList.getVirtualItems()
    : files.map((_, index) => ({ index, start: index * FILE_ROW_HEIGHT }));
  const openFile =
    onOpenFile === undefined || activeFile.status === "D"
      ? undefined
      : () => onOpenFile(activeFile.path);
  return (
    <div className="flex min-h-0 min-w-0 flex-1 flex-col">
      <div className="border-b px-3 py-2 text-xs text-muted-foreground">
        {t("diff.largeChanges")}
      </div>
      {overview.isError && (
        <div
          role="alert"
          className="border-b px-3 py-2 text-xs text-destructive"
        >
          {localizeContractError(overview.error, t)}
        </div>
      )}
      <div className="flex min-h-0 flex-1">
        <div
          ref={scrollRef}
          role="group"
          tabIndex={0}
          aria-label={t("diff.changedFilesNavigation")}
          className="w-[min(36%,15rem)] shrink-0 overflow-auto border-r"
        >
          <div
            className="relative"
            style={{
              height: virtualized
                ? fileList.getTotalSize()
                : files.length * FILE_ROW_HEIGHT,
            }}
          >
            {rows.map((item) => {
              const file = files[item.index]!;
              return (
                <button
                  key={file.path}
                  type="button"
                  aria-pressed={activeFile.path === file.path}
                  title={
                    file.oldPath === null
                      ? file.path
                      : `${file.oldPath} → ${file.path}`
                  }
                  className="absolute inset-x-0 top-0 flex h-8 items-center gap-2 px-2 text-left text-xs hover:bg-muted aria-pressed:bg-accent"
                  style={{ transform: `translateY(${item.start}px)` }}
                  onClick={() =>
                    setSelection({
                      path: file.path,
                      requestId: fileRequest?.requestId,
                    })
                  }
                >
                  <span className="shrink-0 text-muted-foreground">
                    {file.status}
                  </span>
                  <span className="truncate">{file.path}</span>
                </button>
              );
            })}
          </div>
        </div>
        <div
          className="flex min-h-0 min-w-0 flex-1 flex-col"
          aria-busy={preview.isFetching}
        >
          {preview.isPending ? (
            <DiffLoadingState />
          ) : preview.isError ? (
            <DiffMessage
              title={t(tooLarge ? "diff.fileTooLarge" : "diff.fileLoadError")}
              detail={t(
                tooLarge
                  ? "diff.fileTooLargeDetail"
                  : "diff.fileLoadErrorDetail",
                { path: activeFile.path },
              )}
              action={
                tooLarge ? (
                  openFile && (
                    <Button onClick={openFile}>{t("diff.openInFiles")}</Button>
                  )
                ) : (
                  <Button onClick={() => void preview.refetch()}>
                    {t("diff.retry")}
                  </Button>
                )
              }
            />
          ) : previewOutdated ? (
            <DiffMessage
              title={t("diff.comparisonChanged")}
              detail={t("diff.comparisonChangedDetail")}
              action={
                <Button
                  onClick={() =>
                    void Promise.all([overview.refetch(), preview.refetch()])
                  }
                >
                  {t("diff.retry")}
                </Button>
              }
            />
          ) : parsed.length === 0 ? (
            <DiffMessage
              title={t("diff.noChanges")}
              detail={t("diff.noChangesDetail")}
            />
          ) : (
            parsed.map((file, index) => (
              <TaskDiffFocusBody
                key={`${activeFile.path}:${index}`}
                file={file}
                fileFlash={null}
                viewType={viewType}
                targetLine={target?.line}
                targetEndLine={target?.endLine}
                targetSide={target?.side}
              />
            ))
          )}
        </div>
      </div>
    </div>
  );
}
