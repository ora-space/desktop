import { useEffect, useMemo, useRef, useState } from "react";
import type { FileData } from "react-diff-view";
import { Decoration, Diff, Hunk } from "react-diff-view";
import "react-diff-view/style/index.css";
import type { RepositoryCommitDetails } from "@ora/contracts";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@ora/ui";
import { IconChevronDown, IconFileDiff } from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";
import { buildCollapsedDiffSegments } from "../diff/task-diff-collapse";
import { countChanges, parseTaskDiffPatch } from "../diff/task-diff-data";
import { diffFilePath } from "../diff/task-diff-file-tree-utils";
import "../diff/task-diff-view.css";

interface RepositoryCommitDiffViewProps {
  commit?: RepositoryCommitDetails;
  patch?: string;
  loading: boolean;
  error: Error | null;
  initialPath: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

interface RepositoryPatchViewProps {
  patch: string;
  emptyMessage: string;
  initialPath?: string | null;
  className?: string;
}

/** Shows one historical commit patch in a read-only dialog using the existing diff renderer. */
export function RepositoryCommitDiffView({
  commit,
  patch,
  loading,
  error,
  initialPath,
  open,
  onOpenChange,
}: RepositoryCommitDiffViewProps) {
  const { t } = useTranslation();

  if (commit === undefined || !open) return null;

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        className="flex min-h-0 max-w-none flex-col gap-0 overflow-hidden rounded-xl border border-border/70 bg-background p-0 shadow-2xl resize"
        style={{
          width: "min(92vw, 1400px)",
          height: "min(86dvh, 900px)",
          minWidth: "min(640px, calc(100vw - 2rem))",
          minHeight: "min(480px, calc(100dvh - 2rem))",
          maxWidth: "calc(100vw - 2rem)",
          maxHeight: "calc(100dvh - 2rem)",
        }}
      >
        <DialogHeader className="shrink-0 border-b border-border px-4 py-3 text-left">
          <DialogTitle>{t("repository.commitDiff")}</DialogTitle>
          <DialogDescription className="truncate font-mono text-xs">
            {commit.subject} · {commit.shortId}
          </DialogDescription>
        </DialogHeader>
        {loading ? (
          <div className="flex min-h-48 flex-1 items-center justify-center p-6 text-sm text-muted-foreground" role="status">
            {t("diff.loading")}
          </div>
        ) : error ? (
          <div className="flex min-h-48 flex-1 items-center justify-center p-6 text-sm text-destructive" role="alert">
            {localizeContractError(error, t)}
          </div>
        ) : (
          <RepositoryPatchView
            patch={patch ?? ""}
            emptyMessage={t("repository.noDiff")}
            initialPath={initialPath}
            className="min-h-0 min-w-0 flex-1 overflow-auto bg-background"
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

/** Renders a bounded unified patch for either a historical commit or the main checkout. */
export function RepositoryPatchView({
  patch,
  emptyMessage,
  initialPath = null,
  className = "min-h-0 min-w-0 flex-1 overflow-auto bg-background",
}: RepositoryPatchViewProps) {
  const fileElements = useRef(new Map<string, HTMLElement>());
  const parsedFiles = useMemo(() => parseTaskDiffPatch(patch), [patch]);
  const files = useMemo(() => {
    if (initialPath === null) return parsedFiles;
    const target = parsedFiles.find((file) => diffFilePath(file) === initialPath);
    return target === undefined ? parsedFiles : [target];
  }, [initialPath, parsedFiles]);

  useEffect(() => {
    if (initialPath === null) return;
    const target = files.find((file) => diffFilePath(file) === initialPath);
    if (target === undefined) return;
    fileElements.current.get(diffFilePath(target))?.scrollIntoView({ block: "start" });
  }, [files, initialPath]);

  if (files.length === 0) {
    return (
      <div className="flex min-h-48 items-center justify-center p-6 text-sm text-muted-foreground">
        {emptyMessage}
      </div>
    );
  }

  return (
    <div className={className}>
      {files.map((file, fileIndex) => (
        <RepositoryCommitDiffFile
          key={`${file.oldPath}-${file.newPath}-${fileIndex}`}
          file={file}
          fileIndex={fileIndex}
          elementRef={(element) => {
            const path = diffFilePath(file);
            if (element === null) fileElements.current.delete(path);
            else fileElements.current.set(path, element);
          }}
        />
      ))}
    </div>
  );
}

interface RepositoryCommitDiffFileProps {
  file: FileData;
  fileIndex: number;
  elementRef: (element: HTMLElement | null) => void;
}

/** Renders one commit file with the same unified diff primitives used by task review. */
function RepositoryCommitDiffFile({ file, fileIndex, elementRef }: RepositoryCommitDiffFileProps) {
  const { t } = useTranslation();
  const [expandedBlocks, setExpandedBlocks] = useState<Set<string>>(() => new Set());
  const stats = countChanges([file]);
  const renderSegments = useMemo(
    () => buildCollapsedDiffSegments(file.hunks, expandedBlocks),
    [expandedBlocks, file.hunks],
  );

  return (
    <article ref={elementRef} className="border-b border-border bg-background">
      <header className="sticky top-0 z-10 flex min-h-10 items-center gap-2 border-b border-border/60 bg-background/95 px-3 py-2 backdrop-blur">
        <span className="flex size-6 shrink-0 items-center justify-center rounded-md bg-violet-500/12 text-violet-700 ring-1 ring-inset ring-violet-500/15 dark:text-violet-300">
          <IconFileDiff className="size-3.5" />
        </span>
        <span className="min-w-0 flex-1 truncate font-mono text-xs" title={diffFilePath(file)}>
          {diffFilePath(file)}
        </span>
        <span className="shrink-0 text-xs tabular-nums text-emerald-600">+{stats.additions}</span>
        <span className="shrink-0 text-xs tabular-nums text-red-600">-{stats.deletions}</span>
      </header>
      {file.hunks.length === 0 ? (
        <div className="px-4 py-8 text-center text-xs text-muted-foreground">
          {file.isBinary ? t("diff.binary") : t("diff.metadataOnly")}
        </div>
      ) : (
        <div className={`ora-task-diff ora-task-diff--unified ora-task-diff--${file.type} overflow-x-auto`}>
          <Diff
            viewType="unified"
            diffType={file.type}
            hunks={file.hunks}
            optimizeSelection
          >
            {() => renderSegments.map((segment) => (
              segment.kind === "hunk" ? (
                <Hunk key={`${fileIndex}-${segment.key}`} hunk={segment.hunk} />
              ) : (
                <Decoration
                  key={`${fileIndex}-${segment.key}`}
                  className="ora-diff-collapsed"
                  contentClassName="ora-diff-collapsed-cell"
                >
                  <button
                    type="button"
                    className="group flex h-8 w-full items-center justify-center gap-2 text-[11px] text-muted-foreground outline-none transition-colors hover:bg-violet-500/8 hover:text-foreground focus-visible:bg-violet-500/10 focus-visible:text-foreground"
                    aria-label={t("diff.expandUnchanged", { count: segment.lineCount })}
                    onClick={() => {
                      setExpandedBlocks((current) => {
                        const next = new Set(current);
                        next.add(segment.key);
                        return next;
                      });
                    }}
                  >
                    <span className="flex size-5 items-center justify-center rounded-md bg-violet-500/10 text-violet-700 transition-colors group-hover:bg-violet-500/15 dark:text-violet-300">
                      <IconChevronDown className="size-3.5" />
                    </span>
                    {t("diff.unchangedLinesHidden", { count: segment.lineCount })}
                  </button>
                </Decoration>
              )
            ))}
          </Diff>
        </div>
      )}
    </article>
  );
}
