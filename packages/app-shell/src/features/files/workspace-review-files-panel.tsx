import { useState, type ReactNode } from "react";
import { Button } from "@ora/ui";
import { IconFolderOpen, IconRefresh, IconSearch } from "@tabler/icons-react";
import { useQueryClient } from "@tanstack/react-query";
import { useTranslation } from "react-i18next";
import { queryKeys } from "../../state/hooks/query-keys";
import {
  WorkspaceFilesView,
  type WorkspaceDirectoryRequest,
  type WorkspaceArtifactRequest,
  type WorkspaceFileRequest,
} from "./workspace-files-view";

export type FilesSurface = "explorer" | "search";

interface WorkspaceReviewFilesPanelProps {
  projectId: string;
  taskId?: string;
  toolbar?: ReactNode;
  fileRequest?: WorkspaceFileRequest;
  onPreviewPathChange?: (path: string) => void;
  directoryRequest?: WorkspaceDirectoryRequest;
  artifactRequest?: WorkspaceArtifactRequest;
}

/** Hosts project/task file browsing in one review panel. */
export function WorkspaceReviewFilesPanel({
  projectId,
  taskId,
  toolbar,
  fileRequest,
  onPreviewPathChange,
  directoryRequest,
  artifactRequest,
}: WorkspaceReviewFilesPanelProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [surface, setSurface] = useState<FilesSurface>("explorer");
  const [appliedFileRequestId, setAppliedFileRequestId] = useState<
    number | null
  >(null);
  const [appliedDirectoryRequestId, setAppliedDirectoryRequestId] = useState<
    number | null
  >(null);
  const [appliedArtifactRequestId, setAppliedArtifactRequestId] = useState<
    number | null
  >(null);

  if (
    fileRequest !== undefined &&
    fileRequest.requestId !== appliedFileRequestId
  ) {
    setAppliedFileRequestId(fileRequest.requestId);
    setSurface("explorer");
  }
  if (
    artifactRequest !== undefined &&
    artifactRequest.requestId !== appliedArtifactRequestId
  ) {
    setAppliedArtifactRequestId(artifactRequest.requestId);
    setSurface("explorer");
  }
  if (
    directoryRequest !== undefined &&
    directoryRequest.requestId !== appliedDirectoryRequestId
  ) {
    setAppliedDirectoryRequestId(directoryRequest.requestId);
    setSurface("explorer");
  }

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
          aria-pressed={surface === "explorer"}
          onClick={() => setSurface("explorer")}
        >
          <IconFolderOpen />
          {t("files.explorer")}
        </Button>
        <Button
          size="sm"
          variant={surface === "search" ? "secondary" : "ghost"}
          aria-pressed={surface === "search"}
          onClick={() => setSurface("search")}
        >
          <IconSearch />
          {t("files.search")}
        </Button>
        <div className="flex-1" />
        <Button
          size="icon-sm"
          variant="ghost"
          aria-label={t("files.refresh")}
          onClick={refreshFiles}
        >
          <IconRefresh />
        </Button>
        {toolbar}
      </header>
      <div className="min-h-0 flex-1">
        <WorkspaceFilesView
          projectId={projectId}
          taskId={taskId}
          surface={surface}
          hideHeader
          fileRequest={fileRequest}
          onPreviewPathChange={onPreviewPathChange}
          directoryRequest={directoryRequest}
          artifactRequest={artifactRequest}
        />
      </div>
    </section>
  );
}
