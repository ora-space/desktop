import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  WorkspaceEntry,
  WorkspaceSearchKind,
  WorkspaceSearchResult,
} from "@ora/contracts";
import {
  Button,
  Input,
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  ScrollArea,
  toast,
} from "@ora/ui";
import {
  IconChevronDown,
  IconChevronRight,
  IconCodeDots,
  IconFileSearch,
  IconFolder,
  IconFolderOpen,
  IconRefresh,
  IconSearch,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { localizeContractError } from "../../i18n/contract-error";
import { useContractsClient } from "../../contracts-client-context";
import { displayPath } from "../chat/turn-diff-files";
import {
  isAbsoluteWorkspacePath,
  normalizeDiffPath,
  stripTaskCwdPrefix,
} from "../../lib/workspace-path";
import { useTaskWorkspace } from "../../state/hooks/use-task-workspace";
import { useProjects } from "../../state/hooks/use-projects";
import { useComposerFileContextStore } from "../../state/stores/composer-file-context-store";
import { conversationKeyFor } from "../../state/stores/conversation-key";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import {
  WorkspaceFileViewer,
  type WorkspaceFileLineSelection,
  type WorkspaceFileMatchTarget,
} from "./workspace-file-viewer";
import {
  WorkspaceFileIcon,
  workspaceFileVisual,
} from "./workspace-file-visuals";
import { watchWorkspaceContinuously } from "./workspace-watch";
import {
  directoryQueryKey,
  fileQueryKey,
  filesScopeApi,
  filesScopeQueryKey,
  invalidateScopedFileQueries,
  resolveFilesScope,
  searchQueryKey,
  type FilesScope,
} from "./files-scope";

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
}

/** External Files-panel open request. requestId must change to re-apply the same path. */
export interface WorkspaceFileRequest {
  path: string;
  requestId: number;
  line?: number;
  column?: number;
}

interface DirectoryTreeProps {
  scope: FilesScope;
  /** Shared scope API from the parent; avoids re-creating it in every recursive node. */
  scopeApi: ReturnType<typeof filesScopeApi>;
  path: string;
  depth: number;
  expanded: ReadonlySet<string>;
  selectedPath: string | null;
  onToggleDirectory: (path: string) => void;
  onSelectFile: (path: string) => void;
}

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
  const projectsQuery = useProjects({ enabled: scope.kind === "project" });
  const cwd =
    scope.kind === "task"
      ? workspaceQuery.data?.rootPath
      : projectsQuery.data?.find((project) => project.id === projectId)
          ?.rootPath;
  // Absolute ACP paths need the checkout root before we consume requestId; otherwise
  // a later cwd load cannot re-strip and readWorkspaceFile/readProjectFile reject roots.
  // A failed checkout query never yields a root, so treat pending and error alike:
  // keep deferring until cwd resolves instead of feeding an unstripped absolute path.
  const checkoutPending =
    scope.kind === "task"
      ? workspaceQuery.isPending || workspaceQuery.isError
      : projectsQuery.isPending || projectsQuery.isError;
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
  const [selectedTarget, setSelectedTarget] =
    useState<WorkspaceFileMatchTarget | null>(null);
  const [appliedFileRequestId, setAppliedFileRequestId] = useState<
    number | null
  >(null);
  const [searchKind, setSearchKind] = useState<WorkspaceSearchKind>("files");
  const [searchText, setSearchText] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [fileFilterText, setFileFilterText] = useState("");
  const [debouncedFileFilter, setDebouncedFileFilter] = useState("");

  if (
    fileRequest !== undefined &&
    fileRequest.requestId !== appliedFileRequestId
  ) {
    const rawPath = fileRequest.path;
    const absolute =
      isAbsoluteWorkspacePath(rawPath) ||
      isAbsoluteWorkspacePath(normalizeDiffPath(displayPath(rawPath)));
    // Defer absolute-path stripping until the checkout root resolves; a later cwd
    // load re-processes the same requestId once isPending/isError clear.
    const checkoutDeferred = absolute && checkoutPending && !cwd;
    if (!checkoutDeferred) {
      setAppliedFileRequestId(fileRequest.requestId);
      const stripped = cwd
        ? (stripTaskCwdPrefix(rawPath, cwd) ??
          stripTaskCwdPrefix(normalizeDiffPath(rawPath), cwd))
        : null;
      const targetPath = stripped ?? normalizeDiffPath(displayPath(rawPath));
      setSelectedPath(targetPath);
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
            },
      );
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
  const addLineSelectionToChat = (selection: WorkspaceFileLineSelection) => {
    const conversationKey = conversationKeyFor(
      useWorkspaceSelectionStore.getState().selection,
    );
    if (conversationKey === "__none__") {
      toast.warning(t("files.lineSelectionNeedsChat"));
      return;
    }
    useComposerFileContextStore
      .getState()
      .addSelection(conversationKey, selection);
    toast.success(
      t("files.lineSelectionAdded", {
        startLine: selection.startLine,
        endLine: selection.endLine,
      }),
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
  const refresh = () =>
    queryClient.invalidateQueries({
      queryKey: filesScopeQueryKey(scope),
    });

  const body = (
    <div className="min-h-0 flex-1">
      <ResizablePanelGroup orientation="horizontal" className="min-h-0">
        <ResizablePanel id="workspace-file-content" minSize={280}>
          <div className="flex h-full min-w-0 flex-col">
            {selectedPath === null ? (
              <div className="flex flex-1 items-center justify-center text-sm text-muted-foreground">
                {t("files.selectFile")}
              </div>
            ) : (
              <>
                <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border px-3">
                  <WorkspaceFileIcon path={selectedPath} />
                  <span className="truncate font-mono text-xs">
                    {selectedTarget === null
                      ? selectedPath
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
                      onAddLineSelectionToChat={addLineSelectionToChat}
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
                    <DirectoryTree
                      scope={scope}
                      scopeApi={scopeApi}
                      path=""
                      depth={0}
                      expanded={expanded}
                      selectedPath={selectedPath}
                      onToggleDirectory={toggleDirectory}
                      onSelectFile={(path) => {
                        setSelectedPath(path);
                        setSelectedTarget(null);
                      }}
                    />
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

  if (hideHeader) {
    return (
      <section className="flex h-full min-h-0 flex-col bg-background">
        {body}
      </section>
    );
  }

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
    </section>
  );
}

/** Loads one expanded directory lazily and renders its descendants recursively. */
function DirectoryTree({
  scope,
  scopeApi,
  path,
  depth,
  expanded,
  selectedPath,
  onToggleDirectory,
  onSelectFile,
}: DirectoryTreeProps) {
  const { t } = useTranslation();
  const directoryQuery = useQuery({
    queryKey: directoryQueryKey(scope, path),
    queryFn: ({ signal }) => scopeApi.listDirectory(path, signal),
  });

  if (directoryQuery.isLoading) {
    return (
      <p className="px-3 py-2 text-xs text-muted-foreground">
        {t("files.loading")}
      </p>
    );
  }
  if (directoryQuery.error) {
    return (
      <p className="px-3 py-2 text-xs text-destructive">
        {localizeContractError(directoryQuery.error, t)}
      </p>
    );
  }

  return directoryQuery.data?.entries.map((entry) => (
    <WorkspaceTreeEntry
      key={entry.path}
      entry={entry}
      scope={scope}
      scopeApi={scopeApi}
      depth={depth}
      expanded={expanded}
      selectedPath={selectedPath}
      onToggleDirectory={onToggleDirectory}
      onSelectFile={onSelectFile}
    />
  ));
}

/** Renders one tree row and mounts its lazy child query only while expanded. */
function WorkspaceTreeEntry({
  entry,
  scope,
  scopeApi,
  depth,
  expanded,
  selectedPath,
  onToggleDirectory,
  onSelectFile,
}: Omit<DirectoryTreeProps, "path"> & { entry: WorkspaceEntry }) {
  const isDirectory = entry.kind === "directory";
  const isExpanded = isDirectory && expanded.has(entry.path);
  return (
    <>
      <button
        type="button"
        className={`flex h-7 w-full items-center gap-1 border-l-2 pr-2 text-left text-xs hover:bg-muted ${
          selectedPath === entry.path
            ? "border-primary bg-accent/80 text-accent-foreground"
            : "border-transparent"
        }`}
        style={{ paddingLeft: `${8 + depth * 14}px` }}
        onClick={() =>
          isDirectory ? onToggleDirectory(entry.path) : onSelectFile(entry.path)
        }
      >
        {isDirectory ? (
          isExpanded ? (
            <IconChevronDown className="size-3.5" />
          ) : (
            <IconChevronRight className="size-3.5" />
          )
        ) : (
          <span className="w-3.5" />
        )}
        {isDirectory ? (
          <IconFolder className="size-4 shrink-0 text-amber-600" />
        ) : (
          <WorkspaceFileIcon path={entry.path} />
        )}
        <span className="truncate">{entry.name}</span>
      </button>
      {isExpanded && (
        <DirectoryTree
          scope={scope}
          scopeApi={scopeApi}
          path={entry.path}
          depth={depth + 1}
          expanded={expanded}
          selectedPath={selectedPath}
          onToggleDirectory={onToggleDirectory}
          onSelectFile={onSelectFile}
        />
      )}
    </>
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
