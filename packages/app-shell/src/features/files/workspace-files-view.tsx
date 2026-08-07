import { useEffect, useMemo, useState, type ReactNode } from "react";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  ContractsClient,
  WorkspaceEntry,
  WorkspaceFileChange,
  WorkspaceFileEventBatch,
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
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "../../state/hooks/query-keys";
import { useComposerFileContextStore } from "../../state/stores/composer-file-context-store";
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

interface WorkspaceFilesViewProps {
  taskId: string;
  toolbar?: ReactNode;
  hideHeader?: boolean;
  surface?: "explorer" | "search";
  onSurfaceChange?: (surface: "explorer" | "search") => void;
}

interface ProjectFilesViewProps {
  projectId: string;
  rootPath: string;
  branchName?: string;
  toolbar?: ReactNode;
}

type FileExplorerScope =
  | { kind: "project"; projectId: string; rootPath: string; branchName?: string }
  | { kind: "task"; taskId: string };

interface FileExplorerViewProps {
  scope: FileExplorerScope;
  toolbar?: ReactNode;
  hideHeader?: boolean;
  surface?: "explorer" | "search";
  onSurfaceChange?: (surface: "explorer" | "search") => void;
}

interface DirectoryTreeProps {
  scope: FileExplorerScope;
  path: string;
  depth: number;
  expanded: ReadonlySet<string>;
  selectedPath: string | null;
  onToggleDirectory: (path: string) => void;
  onSelectFile: (path: string) => void;
}

const MAX_VISIBLE_SEARCH_RESULTS = 500;

/** Renders the task worktree explorer, ripgrep search, and bounded read-only file viewer. */
export function WorkspaceFilesView({
  taskId,
  toolbar,
  hideHeader = false,
  surface,
  onSurfaceChange,
}: WorkspaceFilesViewProps) {
  return (
    <FileExplorerView
      scope={{ kind: "task", taskId }}
      toolbar={toolbar}
      hideHeader={hideHeader}
      surface={surface}
      onSurfaceChange={onSurfaceChange}
    />
  );
}

/** Renders the project's main checkout with path-scoped directory and file queries. */
export function ProjectFilesView({
  projectId,
  rootPath,
  branchName,
  toolbar,
}: ProjectFilesViewProps) {
  return (
    <FileExplorerView
      scope={{ kind: "project", projectId, rootPath, branchName }}
      toolbar={toolbar}
    />
  );
}

/** Renders a file explorer against either a project checkout or a task worktree. */
function FileExplorerView({
  scope,
  toolbar,
  hideHeader = false,
  surface: controlledSurface,
  onSurfaceChange,
}: FileExplorerViewProps) {
  const { t } = useTranslation();
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const scopeKey = fileScopeKey(scope);
  const [internalSurface, setInternalSurface] = useState<"explorer" | "search">("explorer");
  const surface = controlledSurface ?? internalSurface;
  const setSurface = (next: "explorer" | "search") => {
    if (controlledSurface === undefined) setInternalSurface(next);
    onSurfaceChange?.(next);
  };
  const [expanded, setExpanded] = useState<ReadonlySet<string>>(new Set([""]));
  const [activeDirectory, setActiveDirectory] = useState("");
  const [selectedPath, setSelectedPath] = useState<string | null>(null);
  const [selectedTarget, setSelectedTarget] = useState<WorkspaceFileMatchTarget | null>(
    null,
  );
  const [searchKind, setSearchKind] = useState<WorkspaceSearchKind>("files");
  const [searchText, setSearchText] = useState("");
  const [debouncedSearch, setDebouncedSearch] = useState("");
  const [fileFilterText, setFileFilterText] = useState("");
  const [debouncedFileFilter, setDebouncedFileFilter] = useState("");

  useEffect(() => {
    setExpanded(new Set([""]));
    setActiveDirectory("");
    setSelectedPath(null);
    setSelectedTarget(null);
    setSearchText("");
    setDebouncedSearch("");
    setFileFilterText("");
    setDebouncedFileFilter("");
  }, [scopeKey]);

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedSearch(searchText.trim()), 200);
    return () => clearTimeout(timer);
  }, [searchText]);

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedFileFilter(fileFilterText.trim()), 200);
    return () => clearTimeout(timer);
  }, [fileFilterText]);

  useEffect(() => {
    const controller = new AbortController();
    void watchWorkspaceContinuously({
      signal: controller.signal,
      openStream: (signal) => watchFileScope(client, scope, signal),
      onBatch: (batch) => invalidateFileQueries(queryClient, scope, batch.changes),
    });
    return () => controller.abort();
  }, [client, queryClient, scopeKey]);

  const fileQuery = useQuery({
    queryKey: fileQueryKey(scope, selectedPath ?? ""),
    queryFn: () => readFile(client, scope, selectedPath!),
    enabled: selectedPath !== null,
    refetchOnMount: "always",
  });
  const searchQuery = useQuery({
    queryKey: searchQueryKey(scope, searchKind, debouncedSearch),
    queryFn: ({ signal }) => searchFiles(client, scope, debouncedSearch, searchKind, signal),
    enabled: surface === "search" && debouncedSearch.length > 0,
  });
  const visibleSearchResults = useMemo(
    () => searchQuery.data?.results.slice(0, MAX_VISIBLE_SEARCH_RESULTS) ?? [],
    [searchQuery.data],
  );
  const fileFilterQuery = useQuery({
    queryKey: searchQueryKey(scope, "files", debouncedFileFilter),
    queryFn: ({ signal }) => searchFiles(client, scope, debouncedFileFilter, "files", signal),
    enabled: surface === "explorer" && debouncedFileFilter.length > 0,
  });
  const visibleFileFilterResults = useMemo(
    () => fileFilterQuery.data?.results.slice(0, MAX_VISIBLE_SEARCH_RESULTS) ?? [],
    [fileFilterQuery.data],
  );

  const openSearchResult = (result: WorkspaceSearchResult) => {
    setSelectedPath(result.path);
    setActiveDirectory(parentPath(result.path));
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
  const addLineSelectionToChat = scope.kind === "task"
    ? (selection: WorkspaceFileLineSelection) => {
      useComposerFileContextStore.getState().addSelection(scope.taskId, selection);
      toast.success(t("files.lineSelectionAdded", {
        startLine: selection.startLine,
        endLine: selection.endLine,
      }));
    }
    : undefined;
  const toggleDirectory = (path: string) => {
    setActiveDirectory(path);
    setExpanded((current) => {
      const next = new Set(current);
      if (next.has(path)) next.delete(path);
      else next.add(path);
      return next;
    });
  };
  const refresh = async () => {
    await queryClient.invalidateQueries({
      queryKey: directoryQueryKey(scope, activeDirectory),
    });
    if (selectedPath !== null && parentPath(selectedPath) === activeDirectory) {
      await queryClient.invalidateQueries({ queryKey: fileQueryKey(scope, selectedPath) });
    }
  };

  return (
    <section className="flex h-full min-h-0 flex-col bg-background">
      {!hideHeader && (
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
        <div className="min-w-0 flex-1 px-2">
          {scope.kind === "project" && (
            <div className="truncate text-[10px] text-muted-foreground" title={scope.rootPath}>
              <span className="font-medium text-foreground/80">
                {scope.branchName ?? "project"}
              </span>
              <span className="px-1">·</span>
              {scope.rootPath}
              {activeDirectory.length > 0 && ` / ${activeDirectory}`}
            </div>
          )}
        </div>
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
      )}

      <div className="min-h-0 flex-1">
        <ResizablePanelGroup orientation="horizontal" className="min-h-0">
        <ResizablePanel id="workspace-file-content" minSize={420}>
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
                  <ViewerMessage>{errorMessage(fileQuery.error)}</ViewerMessage>
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
          defaultSize={320}
          minSize={240}
          maxSize={520}
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
                    path=""
                    depth={0}
                    expanded={expanded}
                    selectedPath={selectedPath}
                    onToggleDirectory={toggleDirectory}
                    onSelectFile={(path) => {
                      setSelectedPath(path);
                      setActiveDirectory(parentPath(path));
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
          {surface === "search"
            && searchQuery.data !== undefined
            && (searchQuery.data.truncated
              || searchQuery.data.results.length > MAX_VISIBLE_SEARCH_RESULTS) && (
              <p className="border-t border-border px-3 py-2 text-xs text-muted-foreground">
                {t("files.resultsTruncated")}
              </p>
            )}
        </aside>
        </ResizablePanel>
        </ResizablePanelGroup>
      </div>
    </section>
  );
}

/** Invalidates only the file queries affected by one native event batch. */
async function invalidateFileQueries(
  queryClient: ReturnType<typeof useQueryClient>,
  scope: FileExplorerScope,
  changes: WorkspaceFileChange[],
): Promise<void> {
  const directoryPaths = new Set<string>();
  const filePaths = new Set<string>();
  let invalidateSearch = false;
  let invalidateWorkspace = false;

  for (const change of changes) {
    if (change.kind === "rescanRequired") {
      invalidateWorkspace = true;
      break;
    }
    invalidateSearch = true;
    filePaths.add(change.path);
    directoryPaths.add(parentPath(change.path));
    if (change.kind === "renamed") {
      filePaths.add(change.from);
      directoryPaths.add(parentPath(change.from));
    }
  }

  if (invalidateWorkspace) {
    await queryClient.invalidateQueries({ queryKey: fileScopeQueryKey(scope) });
    return;
  }

  await Promise.all([
    ...Array.from(directoryPaths, (path) =>
      queryClient.invalidateQueries({ queryKey: directoryQueryKey(scope, path) }),
    ),
    ...Array.from(filePaths, (path) =>
      queryClient.invalidateQueries({ queryKey: fileQueryKey(scope, path) }),
    ),
    ...(invalidateSearch
      ? [queryClient.invalidateQueries({ queryKey: searchScopeQueryKey(scope) })]
      : []),
  ]);
}

/** Returns the stable cache namespace for one project or task file scope. */
function fileScopeQueryKey(scope: FileExplorerScope) {
  return scope.kind === "project"
    ? queryKeys.projectFiles(scope.projectId)
    : queryKeys.workspaceFiles(scope.taskId);
}

/** Returns the directory cache key whose path is being rendered or refreshed. */
function directoryQueryKey(scope: FileExplorerScope, path: string) {
  return scope.kind === "project"
    ? queryKeys.projectDirectory(scope.projectId, path)
    : queryKeys.workspaceDirectory(scope.taskId, path);
}

/** Returns the selected-file cache key for one root scope. */
function fileQueryKey(scope: FileExplorerScope, path: string) {
  return scope.kind === "project"
    ? queryKeys.projectFile(scope.projectId, path)
    : queryKeys.workspaceFile(scope.taskId, path);
}

/** Returns the scoped cache key for one filename or content search. */
function searchQueryKey(scope: FileExplorerScope, kind: string, query: string) {
  return scope.kind === "project"
    ? queryKeys.projectSearch(scope.projectId, kind, query)
    : queryKeys.workspaceSearch(scope.taskId, kind, query);
}

/** Returns the search cache prefix used when native changes can alter results. */
function searchScopeQueryKey(scope: FileExplorerScope) {
  return scope.kind === "project"
    ? ["project-files", scope.projectId, "search"] as const
    : ["workspace-files", scope.taskId, "search"] as const;
}

/** Keeps the file cache and watcher stable while display-only project metadata changes. */
function fileScopeKey(scope: FileExplorerScope): string {
  return scope.kind === "project" ? `project:${scope.projectId}` : `task:${scope.taskId}`;
}

/** Opens the native watcher for the selected project or task root. */
function watchFileScope(
  client: ContractsClient,
  scope: FileExplorerScope,
  signal: AbortSignal,
): AsyncIterable<WorkspaceFileEventBatch> {
  return scope.kind === "project"
    ? client.fileSystem.watchProject({ projectId: scope.projectId }, { signal })
    : client.fileSystem.watchWorkspace({ taskId: scope.taskId }, { signal });
}

/** Lists one immediate directory using the endpoint owned by its root scope. */
function listDirectory(
  client: ContractsClient,
  scope: FileExplorerScope,
  path: string,
) {
  return scope.kind === "project"
    ? client.fileSystem.listProjectDirectory({
      projectId: scope.projectId,
      ...(path === "" ? {} : { path }),
    })
    : client.fileSystem.listWorkspaceDirectory({
      taskId: scope.taskId,
      ...(path === "" ? {} : { path }),
    });
}

/** Reads one selected file using the endpoint owned by its root scope. */
function readFile(
  client: ContractsClient,
  scope: FileExplorerScope,
  path: string,
) {
  return scope.kind === "project"
    ? client.fileSystem.readProjectFile({ projectId: scope.projectId, path })
    : client.fileSystem.readWorkspaceFile({ taskId: scope.taskId, path });
}

/** Searches one root scope while preserving the caller's cancellation signal. */
function searchFiles(
  client: ContractsClient,
  scope: FileExplorerScope,
  query: string,
  kind: WorkspaceSearchKind,
  signal: AbortSignal,
) {
  return scope.kind === "project"
    ? client.fileSystem.searchProject(
      { projectId: scope.projectId, query, kind },
      { signal },
    )
    : client.fileSystem.searchWorkspace(
      { taskId: scope.taskId, query, kind },
      { signal },
    );
}

/** Returns the parent directory for a normalized file-relative path. */
function parentPath(path: string): string {
  const separator = path.lastIndexOf("/");
  return separator <= 0 ? "" : path.slice(0, separator);
}

/** Loads one expanded directory lazily and renders its descendants recursively. */
function DirectoryTree({
  scope,
  path,
  depth,
  expanded,
  selectedPath,
  onToggleDirectory,
  onSelectFile,
}: DirectoryTreeProps) {
  const client = useContractsClient();
  const { t } = useTranslation();
  const directoryQuery = useQuery({
    queryKey: directoryQueryKey(scope, path),
    queryFn: () => listDirectory(client, scope, path),
    refetchOnMount: "always",
  });

  if (directoryQuery.isLoading) {
    return <p className="px-3 py-2 text-xs text-muted-foreground">{t("files.loading")}</p>;
  }
  if (directoryQuery.error) {
    return (
      <p className="px-3 py-2 text-xs text-destructive">
        {errorMessage(directoryQuery.error)}
      </p>
    );
  }

  return directoryQuery.data?.entries.map((entry) => (
    <WorkspaceTreeEntry
      key={entry.path}
      entry={entry}
      scope={scope}
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
          isExpanded ? <IconChevronDown className="size-3.5" /> : <IconChevronRight className="size-3.5" />
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
  if (error) return <ViewerMessage>{errorMessage(error)}</ViewerMessage>;
  if (results.length === 0) return <ViewerMessage>{t("files.noResults")}</ViewerMessage>;
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
        <span className="min-w-0 truncate font-mono text-xs">{result.path}</span>
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

/** Converts unknown query failures to useful read-only viewer copy. */
function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
