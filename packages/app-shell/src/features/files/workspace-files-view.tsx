import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  WorkspaceSearchKind,
  WorkspaceSearchResult,
} from "@ora/contracts";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Input,
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  ScrollArea,
  toast,
} from "@ora/ui";
import {
  IconCodeDots,
  IconFileSearch,
  IconFolderOpen,
  IconRefresh,
  IconSearch,
  IconTrash,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";
import { useContractErrorToast } from "../../i18n/use-contract-error-toast";
import { useContractsClient } from "../../contracts-client-context";
import { displayPath } from "../chat/turn-diff-files";
import {
  isAbsoluteWorkspacePath,
  normalizeDiffPath,
  stripTaskCwdPrefix,
} from "../../lib/workspace-path";
import { useTaskWorkspace } from "../../state/hooks/use-task-workspace";
import { useWorkspaces } from "../../state/hooks/use-workspaces";
import { useWorkspaceCwd } from "../../state/hooks/use-workspace-cwd";
import {
  WorkspaceFileViewer,
  type WorkspaceFileMatchTarget,
} from "./workspace-file-viewer";
import {
  WorkspaceFileIcon,
  workspaceFileVisual,
} from "./workspace-file-visuals";
import { watchWorkspaceContinuously } from "../../state/data/file-watch";
import {
  directoryQueryKey,
  fileQueryKey,
  filesScopeApi,
  invalidateFilesScope,
  invalidateScopedFileQueries,
  joinWorkspaceChild,
  parentPath,
  resolveFilesScope,
  searchQueryKey,
} from "../../state/data/files";
import {
  DirectoryTree,
  FileExplorerSession,
  FileTreeRootMenu,
  type FileCreateDraft,
  type FileExplorerClipboard,
} from "./workspace-file-tree";
import { isWorkspacePathInside, uniqueCopyName } from "./unique-copy-name";

interface WorkspaceFilesViewProps {
  projectId: string;
  taskId?: string;
  toolbar?: ReactNode;
  hideHeader?: boolean;
  surface?: "explorer" | "search";
  onSurfaceChange?: (surface: "explorer" | "search") => void;
  fileRequest?: WorkspaceFileRequest;
  /** Reports the file currently previewed so review layout can persist it. */
  onPreviewPathChange?: (path: string) => void;
  directoryRequest?: WorkspaceDirectoryRequest;
  artifactRequest?: WorkspaceArtifactRequest;
}

/** External Files-panel open request. requestId must change to re-apply the same path. */
export interface WorkspaceFileRequest {
  path: string;
  requestId: number;
  line?: number;
  column?: number;
  /** Inclusive end of a cited range; omitted for a single line or search match. */
  endLine?: number;
}

/** External Files-panel directory request that expands and selects a tree node. */
export interface WorkspaceDirectoryRequest {
  path: string;
  requestId: number;
}

/** External request whose real file/directory kind is resolved from its parent listing. */
export type WorkspaceArtifactRequest = WorkspaceFileRequest;

const MAX_VISIBLE_SEARCH_RESULTS = 500;

/** Renders the task or project explorer, ripgrep search, and bounded read-only file viewer. */
export function WorkspaceFilesView({
  projectId,
  taskId,
  toolbar,
  hideHeader = false,
  surface: controlledSurface,
  onSurfaceChange,
  fileRequest,
  onPreviewPathChange,
  directoryRequest,
  artifactRequest,
}: WorkspaceFilesViewProps) {
  const { t } = useTranslation();
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const scope = useMemo(
    () => resolveFilesScope(projectId, taskId),
    [projectId, taskId],
  );
  const scopeApi = useMemo(() => filesScopeApi(client, scope), [client, scope]);
  const workspaceQuery = useTaskWorkspace(
    scope.kind === "task" ? scope.taskId : undefined,
  );
  const { data: workspaces = [], isPending: workspacesPending } =
    useWorkspaces();
  const projectWorkspace =
    scope.kind === "project"
      ? workspaces.find(
          (workspace) =>
            workspace.projectId === scope.projectId &&
            workspace.kind === "main",
        )
      : undefined;
  const workspaceCwdQuery = useWorkspaceCwd(projectWorkspace?.id);
  const cwd =
    scope.kind === "task"
      ? workspaceQuery.data?.rootPath
      : workspaceCwdQuery.data;
  // Absolute ACP paths need the checkout root before we consume requestId; otherwise
  // a later cwd load cannot re-strip and readWorkspaceFile/readProjectFile reject roots.
  // A failed checkout query never yields a root, so treat pending and error alike:
  // keep deferring until cwd resolves instead of feeding an unstripped absolute path.
  const checkoutPending =
    scope.kind === "task"
      ? workspaceQuery.isPending || workspaceQuery.isError
      : workspacesPending ||
        workspaceCwdQuery.isPending ||
        workspaceCwdQuery.isError;
  const [internalSurface, setInternalSurface] = useState<"explorer" | "search">(
    "explorer",
  );
  const surface = controlledSurface ?? internalSurface;
  const setSurface = (next: "explorer" | "search") => {
    if (controlledSurface === undefined) setInternalSurface(next);
    onSurfaceChange?.(next);
  };
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set([""]));
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [selectedDirectory, setSelectedDirectory] = useState<string | null>(
    null,
  );
  const [selectedTarget, setSelectedTarget] =
    useState<WorkspaceFileMatchTarget | null>(null);
  const [appliedFileRequestId, setAppliedFileRequestId] = useState<
    number | null
  >(null);
  const [appliedDirectoryRequestId, setAppliedDirectoryRequestId] = useState<
    number | null
  >(null);
  const [appliedArtifactRequestId, setAppliedArtifactRequestId] = useState<
    number | null
  >(null);
  const [pendingArtifact, setPendingArtifact] = useState<{
    path: string;
    line?: number;
    column?: number;
  } | null>(null);
  const [artifactResolutionMessage, setArtifactResolutionMessage] = useState<
    string | null
  >(null);
  const [searchKind, setSearchKind] = useState<WorkspaceSearchKind>("files");
  const [searchText, setSearchText] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [fileFilterText, setFileFilterText] = useState("");
  const [debouncedFileFilter, setDebouncedFileFilter] = useState("");
  const [createDraft, setCreateDraft] = useState<FileCreateDraft | null>(null);
  const [renamePath, setRenamePath] = useState<string | null>(null);
  const [clipboard, setClipboard] = useState<FileExplorerClipboard | null>(
    null,
  );
  const [pendingDelete, setPendingDelete] = useState<{
    path: string;
    name: string;
    kind: "file" | "directory";
  } | null>(null);
  const [deleting, setDeleting] = useState(false);
  const showContractError = useContractErrorToast();

  if (
    fileRequest !== undefined &&
    fileRequest.requestId !== appliedFileRequestId
  ) {
    const rawPath = fileRequest.path;
    const absolute =
      isAbsoluteWorkspacePath(rawPath) ||
      isAbsoluteWorkspacePath(normalizeDiffPath(displayPath(rawPath)));
    // Defer absolute-path stripping until the checkout root resolves; a later cwd
    // load re-processes the same requestId.
    const checkoutDeferred = absolute && checkoutPending && !cwd;
    if (!checkoutDeferred) {
      setAppliedFileRequestId(fileRequest.requestId);
      setPendingArtifact(null);
      setArtifactResolutionMessage(null);
      const stripped = cwd
        ? (stripTaskCwdPrefix(rawPath, cwd) ??
          stripTaskCwdPrefix(normalizeDiffPath(rawPath), cwd))
        : null;
      const targetPath = stripped ?? normalizeDiffPath(displayPath(rawPath));
      setSelectedPath(targetPath);
      setSelectedDirectory(null);
      const parts = targetPath.split("/");
      if (parts.length > 1) {
        setExpanded((prev) => {
          const next = new Set(prev);
          let current = "";
          for (let i = 0; i < parts.length - 1; i++) {
            current = current === "" ? parts[i]! : `${current}/${parts[i]!}`;
            next.add(current);
          }
          return next;
        });
      }
      setSelectedTarget(
        fileRequest.line === undefined
          ? null
          : {
              line: fileRequest.line,
              column: fileRequest.column ?? 1,
              matchedText: "",
              endLine: fileRequest.endLine,
            },
      );
    }
  }

  if (
    artifactRequest !== undefined &&
    artifactRequest.requestId !== appliedArtifactRequestId
  ) {
    const rawPath = artifactRequest.path.replace(/[\\/]+$/, "");
    const absolute =
      isAbsoluteWorkspacePath(rawPath) ||
      isAbsoluteWorkspacePath(normalizeDiffPath(displayPath(rawPath)));
    const checkoutDeferred = absolute && checkoutPending && !cwd;
    if (!checkoutDeferred) {
      setAppliedArtifactRequestId(artifactRequest.requestId);
      setSelectedPath(null);
      setSelectedDirectory(null);
      setSelectedTarget(null);
      setArtifactResolutionMessage(t("files.loading"));
      const stripped = cwd
        ? (stripTaskCwdPrefix(rawPath, cwd) ??
          stripTaskCwdPrefix(normalizeDiffPath(rawPath), cwd))
        : null;
      setPendingArtifact({
        path: stripped ?? normalizeDiffPath(displayPath(rawPath)),
        line: artifactRequest.line,
        column: artifactRequest.column,
      });
      if (controlledSurface === undefined) setInternalSurface("explorer");
    }
  }

  if (
    directoryRequest !== undefined &&
    directoryRequest.requestId !== appliedDirectoryRequestId
  ) {
    const rawPath = directoryRequest.path.replace(/[\\/]+$/, "");
    const absolute =
      isAbsoluteWorkspacePath(rawPath) ||
      isAbsoluteWorkspacePath(normalizeDiffPath(displayPath(rawPath)));
    const checkoutDeferred = absolute && checkoutPending && !cwd;
    if (!checkoutDeferred) {
      setAppliedDirectoryRequestId(directoryRequest.requestId);
      setPendingArtifact(null);
      setArtifactResolutionMessage(null);
      const stripped = cwd
        ? (stripTaskCwdPrefix(rawPath, cwd) ??
          stripTaskCwdPrefix(normalizeDiffPath(rawPath), cwd))
        : null;
      const targetPath = (
        stripped ?? normalizeDiffPath(displayPath(rawPath))
      ).replace(/\/+$/, "");
      setSelectedPath(null);
      setSelectedTarget(null);
      setSelectedDirectory(targetPath);
      if (controlledSurface === undefined) setInternalSurface("explorer");
      setExpanded((prev) => {
        const next = new Set(prev);
        let current = "";
        for (const part of targetPath.split("/")) {
          if (part === "") continue;
          current = current === "" ? part : `${current}/${part}`;
          next.add(current);
        }
        return next;
      });
    }
  }

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedSearch(searchText.trim()), 200);
    return () => clearTimeout(timer);
  }, [searchText]);

  useEffect(() => {
    const timer = setTimeout(
      () => setDebouncedFileFilter(fileFilterText.trim()),
      200,
    );
    return () => clearTimeout(timer);
  }, [fileFilterText]);

  useEffect(() => {
    if (selectedPath === null) return;
    onPreviewPathChange?.(selectedPath);
  }, [onPreviewPathChange, selectedPath]);

  // A new chat requestId must re-read even when the path is unchanged. Otherwise
  // a file the user deleted after an earlier preview stays on screen from cache.
  const fileRequestId = fileRequest?.requestId;
  useEffect(() => {
    if (fileRequestId === undefined || selectedPath === null) return;
    void queryClient.invalidateQueries({
      queryKey: fileQueryKey(scope, selectedPath),
    });
  }, [fileRequestId, queryClient, scope, selectedPath]);

  useEffect(() => {
    const controller = new AbortController();
    void watchWorkspaceContinuously({
      signal: controller.signal,
      openStream: (signal) => scopeApi.watch(signal),
      onBatch: (batch) =>
        invalidateScopedFileQueries(queryClient, scope, batch.changes),
    });
    return () => controller.abort();
  }, [queryClient, scope, scopeApi]);

  const fileQuery = useQuery({
    queryKey: fileQueryKey(scope, selectedPath ?? ""),
    queryFn: ({ signal }) => scopeApi.readFile(selectedPath!, signal),
    enabled: selectedPath !== null,
  });
  const pendingArtifactParent = pendingArtifact?.path.includes("/")
    ? pendingArtifact.path.slice(0, pendingArtifact.path.lastIndexOf("/"))
    : "";
  const artifactParentQuery = useQuery({
    queryKey: directoryQueryKey(scope, pendingArtifactParent),
    queryFn: ({ signal }) =>
      scopeApi.listDirectory(pendingArtifactParent, signal),
    enabled: pendingArtifact !== null,
    staleTime: 0,
    refetchOnMount: "always",
  });

  if (pendingArtifact !== null && artifactParentQuery.error !== null) {
    setArtifactResolutionMessage(
      localizeContractError(artifactParentQuery.error, t),
    );
    setPendingArtifact(null);
  } else if (
    pendingArtifact !== null &&
    artifactParentQuery.data !== undefined &&
    !artifactParentQuery.isFetching
  ) {
    const entry = artifactParentQuery.data.entries.find(
      (candidate) =>
        candidate.path.toLowerCase() === pendingArtifact.path.toLowerCase(),
    );
    if (entry === undefined) {
      setArtifactResolutionMessage(t("errors.file_system_path_not_found"));
    } else if (entry.kind === "directory") {
      setArtifactResolutionMessage(null);
      setSelectedPath(null);
      setSelectedTarget(null);
      setSelectedDirectory(entry.path);
      setExpanded((current) => {
        const next = new Set(current);
        let path = "";
        for (const part of entry.path.split("/")) {
          path = path === "" ? part : `${path}/${part}`;
          next.add(path);
        }
        return next;
      });
    } else {
      setArtifactResolutionMessage(null);
      setSelectedDirectory(null);
      setSelectedPath(pendingArtifact.path);
      setSelectedTarget(
        pendingArtifact.line === undefined
          ? null
          : {
              line: pendingArtifact.line,
              column: pendingArtifact.column ?? 1,
              matchedText: "",
            },
      );
    }
    setPendingArtifact(null);
  }
  const searchQuery = useQuery({
    queryKey: searchQueryKey(scope, searchKind, debouncedSearch),
    queryFn: ({ signal }) =>
      scopeApi.search(debouncedSearch, searchKind, signal),
    enabled: surface === "search" && debouncedSearch.length > 0,
  });
  const visibleSearchResults = useMemo(
    () => searchQuery.data?.results.slice(0, MAX_VISIBLE_SEARCH_RESULTS) ?? [],
    [searchQuery.data],
  );
  const fileFilterQuery = useQuery({
    queryKey: searchQueryKey(scope, "files", debouncedFileFilter),
    queryFn: ({ signal }) =>
      scopeApi.search(debouncedFileFilter, "files", signal),
    enabled: surface === "explorer" && debouncedFileFilter.length > 0,
  });
  const visibleFileFilterResults = useMemo(
    () =>
      fileFilterQuery.data?.results.slice(0, MAX_VISIBLE_SEARCH_RESULTS) ?? [],
    [fileFilterQuery.data],
  );

  const openSearchResult = (result: WorkspaceSearchResult) => {
    setPendingArtifact(null);
    setArtifactResolutionMessage(null);
    setSelectedDirectory(null);
    setSelectedPath(result.path);
    setSelectedTarget(
      result.kind === "match"
        ? {
            line: result.line,
            column: result.column,
            matchedText: result.matchedText,
          }
        : null,
    );
  };
  const toggleDirectory = (path: string) => {
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };
  const beginCreate = (parent: string, kind: FileCreateDraft["kind"]) => {
    if (parent !== "") {
      setExpanded((current) => new Set(current).add(parent));
    }
    setRenamePath(null);
    setCreateDraft({ parentPath: parent, kind });
  };
  const commitCreate = async (path: string, kind: FileCreateDraft["kind"]) => {
    const created = await scopeApi.createEntry(path, kind);
    setCreateDraft(null);
    await queryClient.invalidateQueries({
      queryKey: directoryQueryKey(scope, parentPath(created.path)),
    });
    if (created.kind === "file") {
      setSelectedDirectory(null);
      setSelectedPath(created.path);
      setSelectedTarget(null);
      return;
    }
    setSelectedPath(null);
    setSelectedTarget(null);
    setSelectedDirectory(created.path);
    setExpanded((current) => new Set(current).add(created.path));
  };
  const selectCreated = (path: string, kind: "file" | "directory") => {
    if (kind === "file") {
      setSelectedDirectory(null);
      setSelectedPath(path);
      setSelectedTarget(null);
      return;
    }
    setSelectedPath(null);
    setSelectedTarget(null);
    setSelectedDirectory(path);
    setExpanded((current) => new Set(current).add(path));
  };
  const commitRename = async (from: string, name: string) => {
    const to = joinWorkspaceChild(parentPath(from), name);
    const moved = await scopeApi.moveEntry(from, to);
    setRenamePath(null);
    await invalidateFilesScope(queryClient, scope);
    selectCreated(moved.path, moved.kind);
  };
  const pasteEntry = async (
    targetPath: string,
    targetKind: "file" | "directory" | "root",
  ) => {
    if (clipboard === null) return;
    const destParent =
      targetKind === "file" ? parentPath(targetPath) : targetPath;
    if (
      clipboard.path !== "" &&
      isWorkspacePathInside(clipboard.path, destParent)
    ) {
      toast.error(t("files.pasteIntoSelf"));
      return;
    }
    if (clipboard.mode === "cut" && parentPath(clipboard.path) === destParent) {
      setClipboard(null);
      return;
    }
    const listing =
      queryClient.getQueryData<{
        path: string;
        entries: Array<{ name: string }>;
      }>(directoryQueryKey(scope, destParent)) ??
      (await scopeApi.listDirectory(destParent));
    const occupied = new Set(listing.entries.map((entry) => entry.name));
    const baseName = clipboard.path.split("/").pop() ?? clipboard.path;
    const destPath = joinWorkspaceChild(
      destParent,
      uniqueCopyName(baseName, occupied),
    );
    try {
      const relocated =
        clipboard.mode === "copy"
          ? await scopeApi.copyEntry(clipboard.path, destPath)
          : await scopeApi.moveEntry(clipboard.path, destPath);
      if (clipboard.mode === "cut") setClipboard(null);
      if (destParent !== "") {
        setExpanded((current) => new Set(current).add(destParent));
      }
      await invalidateFilesScope(queryClient, scope);
      selectCreated(relocated.path, relocated.kind);
    } catch (cause) {
      showContractError(cause);
    }
  };
  const confirmDelete = async () => {
    if (pendingDelete === null || deleting) return;
    setDeleting(true);
    try {
      await scopeApi.deleteEntry(pendingDelete.path);
      if (
        selectedPath !== null &&
        isWorkspacePathInside(pendingDelete.path, selectedPath)
      ) {
        setSelectedPath(null);
        setSelectedTarget(null);
      }
      if (
        selectedDirectory !== null &&
        isWorkspacePathInside(pendingDelete.path, selectedDirectory)
      ) {
        setSelectedDirectory(null);
      }
      if (
        clipboard !== null &&
        isWorkspacePathInside(pendingDelete.path, clipboard.path)
      ) {
        setClipboard(null);
      }
      setPendingDelete(null);
      await invalidateFilesScope(queryClient, scope);
    } catch (cause) {
      showContractError(cause);
    } finally {
      setDeleting(false);
    }
  };
  const refresh = () => invalidateFilesScope(queryClient, scope);

  const body = (
    <div className="min-h-0 flex-1">
      <ResizablePanelGroup orientation="horizontal" className="min-h-0">
        <ResizablePanel id="workspace-file-content" minSize={280}>
          <div className="flex h-full min-w-0 flex-col">
            {selectedPath === null ? (
              <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
                {artifactResolutionMessage ?? t("files.selectFile")}
              </div>
            ) : (
              <>
                <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
                  <WorkspaceFileIcon path={selectedPath} />
                  <span className="truncate font-mono text-xs">
                    {selectedTarget === null
                      ? selectedPath
                      : selectedTarget.endLine !== undefined &&
                          selectedTarget.endLine !== selectedTarget.line
                        ? `${selectedPath}:${selectedTarget.line}-${selectedTarget.endLine}`
                        : `${selectedPath}:${selectedTarget.line}:${selectedTarget.column}`}
                  </span>
                  {fileQuery.data && (
                    <div className="ml-auto flex shrink-0 items-center gap-2 pl-3">
                      <span className="rounded border border-border bg-muted/60 px-1.5 py-0.5 font-mono text-[9px] font-medium tracking-wide text-muted-foreground">
                        {workspaceFileVisual(selectedPath).label}
                      </span>
                      <span className="text-[11px] text-muted-foreground">
                        {fileQuery.data.sizeBytes.toLocaleString()} B
                      </span>
                    </div>
                  )}
                </div>
                <div className="flex min-h-0 flex-1 flex-col">
                  {fileQuery.isLoading ? (
                    <ViewerMessage>{t("files.loading")}</ViewerMessage>
                  ) : fileQuery.error ? (
                    <ViewerMessage>
                      {localizeContractError(fileQuery.error, t)}
                    </ViewerMessage>
                  ) : (
                    <WorkspaceFileViewer
                      key={selectedPath}
                      content={fileQuery.data?.content ?? ""}
                      path={selectedPath}
                      target={selectedTarget}
                      onDismissJump={() => setSelectedTarget(null)}
                    />
                  )}
                </div>
              </>
            )}
          </div>
        </ResizablePanel>
        <ResizableHandle
          withHandle
          aria-label={t("files.resizePanel")}
          title={t("files.resizePanel")}
          className="z-10 transition-colors hover:bg-ring focus-visible:bg-ring"
        />
        <ResizablePanel
          id="workspace-file-tree"
          defaultSize={260}
          minSize={180}
          maxSize={480}
          className="border-l border-border"
        >
          <aside className="flex h-full min-w-0 flex-col">
            {surface === "search" && (
              <div className="space-y-2 border-b border-border p-2">
                <Input
                  value={searchText}
                  onChange={(event) => setSearchText(event.target.value)}
                  placeholder={t("files.searchPlaceholder")}
                  aria-label={t("files.search")}
                  autoFocus
                />
                <div className="flex gap-1">
                  <Button
                    size="sm"
                    variant={searchKind === "files" ? "secondary" : "ghost"}
                    onClick={() => setSearchKind("files")}
                  >
                    <IconFileSearch />
                    {t("files.searchFiles")}
                  </Button>
                  <Button
                    size="sm"
                    variant={searchKind === "content" ? "secondary" : "ghost"}
                    onClick={() => setSearchKind("content")}
                  >
                    <IconCodeDots />
                    {t("files.searchContent")}
                  </Button>
                </div>
              </div>
            )}
            {surface === "explorer" && (
              <div className="border-b border-border p-2">
                <Input
                  value={fileFilterText}
                  onChange={(event) => setFileFilterText(event.target.value)}
                  placeholder={t("files.filterFiles")}
                  aria-label={t("files.filterFiles")}
                />
              </div>
            )}
            <ScrollArea className="min-h-0 flex-1">
              <div className="py-1">
                {surface === "explorer" ? (
                  debouncedFileFilter.length > 0 ? (
                    <SearchResults
                      results={visibleFileFilterResults}
                      loading={fileFilterQuery.isFetching}
                      error={fileFilterQuery.error}
                      selectedPath={selectedPath}
                      onSelect={openSearchResult}
                    />
                  ) : (
                    <div className="flex min-h-full flex-col">
                      <FileExplorerSession
                        value={{
                          cwd,
                          clipboard,
                          createDraft,
                          renamePath,
                          onBeginCreate: beginCreate,
                          onCommitCreate: commitCreate,
                          onCancelCreate: () => setCreateDraft(null),
                          onBeginRename: (path) => {
                            setCreateDraft(null);
                            setRenamePath(path);
                          },
                          onCommitRename: commitRename,
                          onCancelRename: () => setRenamePath(null),
                          onCopy: (path) =>
                            setClipboard({ mode: "copy", path }),
                          onCut: (path) => setClipboard({ mode: "cut", path }),
                          onPaste: (targetPath, targetKind) => {
                            void pasteEntry(targetPath, targetKind);
                          },
                          onDelete: (path, name, kind) =>
                            setPendingDelete({ path, name, kind }),
                        }}
                      >
                        <DirectoryTree
                          scope={scope}
                          scopeApi={scopeApi}
                          path=""
                          depth={0}
                          expanded={expanded}
                          selectedPath={selectedDirectory ?? selectedPath}
                          onToggleDirectory={(path) => {
                            setPendingArtifact(null);
                            setArtifactResolutionMessage(null);
                            setSelectedPath(null);
                            setSelectedTarget(null);
                            setSelectedDirectory(path);
                            toggleDirectory(path);
                          }}
                          onSelectFile={(path) => {
                            setPendingArtifact(null);
                            setArtifactResolutionMessage(null);
                            setSelectedDirectory(null);
                            setSelectedPath(path);
                            setSelectedTarget(null);
                          }}
                        />
                        <FileTreeRootMenu>
                          <div className="min-h-8 flex-1" />
                        </FileTreeRootMenu>
                      </FileExplorerSession>
                    </div>
                  )
                ) : (
                  <SearchResults
                    results={visibleSearchResults}
                    loading={searchQuery.isFetching}
                    error={searchQuery.error}
                    selectedPath={selectedPath}
                    onSelect={openSearchResult}
                  />
                )}
              </div>
            </ScrollArea>
            {surface === "search" &&
              searchQuery.data !== undefined &&
              (searchQuery.data.truncated ||
                searchQuery.data.results.length >
                  MAX_VISIBLE_SEARCH_RESULTS) && (
                <p className="border-t border-border px-3 py-2 text-xs text-muted-foreground">
                  {t("files.resultsTruncated")}
                </p>
              )}
          </aside>
        </ResizablePanel>
      </ResizablePanelGroup>
    </div>
  );

  const deleteDialog = (
    <AlertDialog
      open={pendingDelete !== null}
      onOpenChange={(open) => {
        if (!open && !deleting) setPendingDelete(null);
      }}
    >
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>
            {t("files.deleteTitle", { name: pendingDelete?.name ?? "" })}
          </AlertDialogTitle>
          <AlertDialogDescription>
            {pendingDelete?.kind === "directory"
              ? t("files.deleteFolderDescription")
              : t("files.deleteFileDescription")}
          </AlertDialogDescription>
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={deleting}>
            {t("common.cancel")}
          </AlertDialogCancel>
          <AlertDialogAction
            variant="destructive"
            disabled={deleting}
            onClick={(event) => {
              event.preventDefault();
              void confirmDelete();
            }}
          >
            <IconTrash />
            {deleting ? t("files.deleting") : t("common.delete")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );

  if (hideHeader) {
    return (
      <section className="flex h-full min-h-0 flex-col bg-background">
        {body}
        {deleteDialog}
      </section>
    );
  }

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
          className="shrink-0"
          aria-label={t("files.refresh")}
          onClick={() => void refresh()}
        >
          <IconRefresh />
        </Button>
        {toolbar}
      </header>
      {body}
      {deleteDialog}
    </section>
  );
}

/** Renders the bounded filename or line-match result collection. */
function SearchResults({
  results,
  loading,
  error,
  selectedPath,
  onSelect,
}: {
  results: WorkspaceSearchResult[];
  loading: boolean;
  error: Error | null;
  selectedPath: string | null;
  onSelect: (result: WorkspaceSearchResult) => void;
}) {
  const { t } = useTranslation();
  if (loading) return <ViewerMessage>{t("files.searching")}</ViewerMessage>;
  if (error)
    return <ViewerMessage>{localizeContractError(error, t)}</ViewerMessage>;
  if (results.length === 0)
    return <ViewerMessage>{t("files.noResults")}</ViewerMessage>;
  return results.map((result, index) => (
    <button
      key={`${result.path}:${result.kind === "match" ? `${result.line}:${result.column}` : index}`}
      type="button"
      className={`block w-full border-l-2 border-b border-b-border/50 px-3 py-2 text-left hover:bg-muted ${
        selectedPath === result.path
          ? "border-l-primary bg-accent/80"
          : "border-l-transparent"
      }`}
      onClick={() => onSelect(result)}
    >
      <span className="flex items-center gap-1.5">
        <WorkspaceFileIcon path={result.path} />
        <span className="min-w-0 truncate font-mono text-xs">
          {result.path}
        </span>
      </span>
      {result.kind === "match" && (
        <>
          <span className="mt-0.5 block text-[10px] text-muted-foreground">
            {result.line}:{result.column}
          </span>
          <span className="mt-1 block truncate font-mono text-[11px] text-muted-foreground">
            {result.preview}
          </span>
        </>
      )}
    </button>
  ));
}

/** Centers lightweight loading, empty, and error copy inside a viewer surface. */
function ViewerMessage({ children }: { children: ReactNode }) {
  return <p className="p-4 text-xs text-muted-foreground">{children}</p>;
}
