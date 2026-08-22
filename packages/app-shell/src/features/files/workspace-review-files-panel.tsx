import { useRef, useState, type ReactNode } from "react";
import { Button } from "@ora/ui";
import {
  IconFileDescription,
  IconFolderOpen,
  IconRefresh,
  IconSearch,
} from "@tabler/icons-react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { queryKeys } from "../../state/hooks/query-keys";
import { SpecsContent, type SpecsContentHandle } from "../specs/specs-view";
import {
  WorkspaceFilesView,
  type WorkspaceFileRequest,
} from "./workspace-files-view";

export type FilesSurface = "explorer" | "search" | "specs";

interface WorkspaceReviewFilesPanelProps {
  projectId: string;
  taskId?: string;
  toolbar?: ReactNode;
  fileRequest?: WorkspaceFileRequest;
  onPreviewPathChange?: (path: string) => void;
}

/** Hosts project/task file browsing and the read-only Spec catalog in one review panel. */
export function WorkspaceReviewFilesPanel({
  projectId,
  taskId,
  toolbar,
  fileRequest,
  onPreviewPathChange,
}: WorkspaceReviewFilesPanelProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [surface, setSurface] = useState<FilesSurface>("explorer");
  const [appliedFileRequestId, setAppliedFileRequestId] = useState<
    number | null
  >(null);
  const specsRef = useRef<SpecsContentHandle>(null);
  const [specsRefreshing, setSpecsRefreshing] = useState(false);

  if (
    fileRequest !== undefined &&
    fileRequest.requestId !== appliedFileRequestId
  ) {
    setAppliedFileRequestId(fileRequest.requestId);
    setSurface("explorer");
  }

  const refreshSpecs = () => void specsRef.current?.refresh();
  const refreshFiles = () => {
    if (taskId !== undefined) {
      void queryClient.invalidateQueries({
        queryKey: queryKeys.workspaceFiles(taskId),
      });
      return;
    }
    void queryClient.invalidateQueries({
      queryKey: queryKeys.projectFiles(projectId),
    });
  };

  return (
    <section className="flex h-full min-h-0 flex-col bg-background">
      <header className="flex h-12 shrink-0 items-center gap-1 border-b border-border px-3">
        <Button
          size="sm"
          variant={surface === "explorer" ? "secondary" : "ghost"}
          onClick={() => setSurface("explorer")}
        >
          <IconFolderOpen />
          {t("files.explorer")}
        </Button>
        <Button
          size="sm"
          variant={surface === "search" ? "secondary" : "ghost"}
          onClick={() => setSurface("search")}
        >
          <IconSearch />
          {t("files.search")}
        </Button>
        <Button
          size="sm"
          variant={surface === "specs" ? "secondary" : "ghost"}
          onClick={() => {
            if (surface === "specs") {
              specsRef.current?.clearSelection();
              return;
            }
            setSurface("specs");
          }}
        >
          <IconFileDescription />
          {t("specs.specs")}
        </Button>
        <div className="flex-1" />
        {surface === "specs" ? (
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t("specs.refresh")}
            onClick={refreshSpecs}
          >
            <IconRefresh
              className={specsRefreshing ? "animate-spin" : undefined}
            />
          </Button>
        ) : (
          <Button
            size="icon-sm"
            variant="ghost"
            aria-label={t("files.refresh")}
            onClick={refreshFiles}
          >
            <IconRefresh />
          </Button>
        )}
        {toolbar}
      </header>
      <div className="min-h-0 flex-1">
        {surface === "specs" ? (
          <SpecsContent
            ref={specsRef}
            projectId={projectId}
            taskId={taskId}
            onRefreshingChange={setSpecsRefreshing}
          />
        ) : (
          <WorkspaceFilesView
            projectId={projectId}
            taskId={taskId}
            surface={surface}
            hideHeader
            fileRequest={fileRequest}
            onPreviewPathChange={onPreviewPathChange}
          />
        )}
      </div>
    </section>
  );
}
