import { useLayoutEffect, useRef, useState } from "react";
import type { FileData } from "react-diff-view";
import { MemoizedTaskDiffFile, type TaskDiffFileProps } from "./task-diff-file";
import { diffFilePath } from "./task-diff-file-tree-utils";
import { TaskDiffScrollArea } from "./task-diff-scroll-area";

export interface TaskDiffFocusBodyProps extends Omit<
  TaskDiffFileProps,
  "file"
> {
  file: FileData | null;
  fileFlash: { path: string; seq: number } | null;
  /** Left-click outside the cited row dismisses the jump highlight. */
  onDismissJumpHighlight?: (event: React.MouseEvent) => void;
}

/**
 * Focused (single-file) body: renders only the selected file so large diffs
 * never pay for the whole list at once. The scroll container carries the
 * `.ora-diff-scroll-region` class so the file's own jump-scroll effect and the
 * pinned-quote lookup keep working, and is handed to the file so a huge file
 * can row-window its body internally (see `task-diff-file.tsx`).
 */
export function TaskDiffFocusBody({
  file,
  fileFlash,
  onDismissJumpHighlight,
  ...fileProps
}: TaskDiffFocusBodyProps) {
  const scrollRegionRef = useRef<HTMLDivElement | null>(null);
  const [scrollElement, setScrollElement] = useState<HTMLElement | null>(null);

  useLayoutEffect(() => {
    setScrollElement(scrollRegionRef.current);
  }, [file]);

  return (
    <TaskDiffScrollArea
      viewportRef={scrollRegionRef}
      onMouseDown={onDismissJumpHighlight}
    >
      {file === null ? null : (
        <div className="flex w-full flex-col pb-6 pl-4">
          <div
            data-diff-path={diffFilePath(file)}
            className="relative scroll-mt-0"
          >
            {/* Keyed by path so a file switch resets the file's own expansion
                state (a scroll body gives each file its own instance). The
                scroll region is passed so the file row-windows itself when it
                is large (see `TaskDiffFile`). */}
            <MemoizedTaskDiffFile
              key={diffFilePath(file)}
              file={file}
              scrollElement={scrollElement}
              {...fileProps}
            />
            {fileFlash !== null && (
              <div
                key={fileFlash.seq}
                aria-hidden="true"
                className="ora-diff-file-flash"
              />
            )}
          </div>
        </div>
      )}
    </TaskDiffScrollArea>
  );
}
