import { useMemo, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import type {
  PullRepositoryOutcome,
  ProjectBranch,
  RepositoryRemoteStatus,
  RepositorySyncOperation,
  RepositoryWorkingTree,
  Task,
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
  Badge,
  Button,
  ScrollArea,
  Tabs,
  TabsContent,
  TabsList,
  TabsTrigger,
} from "@ora/ui";
import {
  IconArrowLeft,
  IconArrowDown,
  IconDownload,
  IconGitBranch,
  IconGitCommit,
  IconGitFork,
  IconGitMerge,
  IconPlus,
  IconRefresh,
  IconUpload,
} from "@tabler/icons-react";
import { useTranslation } from "react-i18next";
import { useProjects } from "../../state/hooks/use-projects";
import { useProjectBranches } from "../../state/hooks/use-project-branches";
import { useRepositorySnapshot } from "../../state/hooks/use-repository-snapshot";
import {
  useCheckoutRepositoryBranch,
  useCreateRepositoryBranch,
} from "../../state/hooks/use-repository-branch-mutations";
import {
  useFetchRepository,
  usePullRepository,
  usePushRepositoryBranch,
  useResolveRepositorySync,
} from "../../state/hooks/use-repository-remote-mutations";
import { queryKeys } from "../../state/hooks/query-keys";
import { useTasks } from "../../state/hooks/use-tasks";
import { useTaskWorkspace } from "../../state/hooks/use-task-workspace";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { localizeContractError } from "../../i18n/contract-error";
import { DragRegion } from "../../components/drag-region";
import { WindowControls } from "../../components/window-controls";
import { TaskDiffView } from "../diff/task-diff-view";
import { ProjectFilesView } from "../files/workspace-files-view";
import { EntityDialog } from "../workspace/entity-dialog";
import { LocationActionsButton } from "../workspace/location-actions-button";
import { RepositoryGraph } from "./repository-graph";
import { RepositoryWorkingTreeDiffView } from "./repository-working-tree-diff-view";

type RepositoryTab = "graph" | "changes" | "files" | "branches";

/** Renders the first repository workspace slice around the selected Ora project. */
export function RepositoryWorkspaceView() {
  const { t } = useTranslation();
  const queryClient = useQueryClient();
  const [tab, setTab] = useState<RepositoryTab>("graph");
  const closeRepository = useUiStore((state) => state.setRepositoryOpen);
  const selection = useWorkspaceSelectionStore((state) => state.selection);
  const projectsQuery = useProjects();
  const tasksQuery = useTasks();
  const project = projectsQuery.data?.find((item) => item.id === selection.projectId);
  const projectTasks = useMemo(
    () => tasksQuery.data?.filter((item) => item.projectId === project?.id) ?? [],
    [project?.id, tasksQuery.data],
  );
  const selectedTask = projectTasks.find((item) => item.id === selection.taskId);
  const selectedWorktreeTask = selectedTask?.workspaceMode === "worktree"
    ? selectedTask
    : undefined;
  const repositoryQuery = useRepositorySnapshot(project?.id ?? null);
  const branchQueryEnabled = tab === "branches"
    || (
      repositoryQuery.data !== undefined
      && !repositoryQuery.data.references.some((reference) => reference.kind === "local")
    );
  const branchesQuery = useProjectBranches(project?.id ?? null, { enabled: branchQueryEnabled });
  const workspaceQuery = useTaskWorkspace(selectedWorktreeTask?.id);
  const branches = branchesQuery.data ?? [];

  if (project === undefined) {
    return (
      <main id="main-content" className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
        <RepositoryHeader
          onRefresh={() => undefined}
          onClose={() => closeRepository(false)}
        />
        <RepositoryEmptyState
          title={t("repository.noProject")}
          description={t("repository.noProjectDescription")}
        />
      </main>
    );
  }

  const activeBranch = workspaceQuery.data?.branchName
    ?? repositoryQuery.data?.currentBranch
    ?? preferredBranch(branches);

  return (
    <main id="main-content" className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
      <RepositoryHeader
        projectId={project.id}
        projectName={project.name}
        projectPath={project.rootPath}
        taskId={selectedWorktreeTask?.id}
        activeBranch={activeBranch}
        refreshing={repositoryQuery.isFetching}
        remoteStatus={repositoryQuery.data?.remoteStatus}
        syncOperation={repositoryQuery.data?.syncOperation}
        onRefresh={() => {
          void repositoryQuery.refetch();
          if (branchQueryEnabled) {
            void branchesQuery.refetch();
          }
          void queryClient.invalidateQueries({
            queryKey: queryKeys.repositoryWorkingTreeDiff(project.id),
          });
          void queryClient.invalidateQueries({
            queryKey: queryKeys.projectFiles(project.id),
          });
        }}
        onClose={() => closeRepository(false)}
      />
      <Tabs
        value={tab}
        onValueChange={(value) => setTab(value as RepositoryTab)}
        className="min-h-0 flex-1"
      >
        <div className="flex h-11 shrink-0 items-center border-b border-border px-4">
          <TabsList variant="line" className="h-8">
            <TabsTrigger value="graph" className="gap-1.5 px-3">
              <IconGitCommit />
              {t("repository.graph")}
            </TabsTrigger>
            <TabsTrigger value="changes" className="gap-1.5 px-3">
              <IconGitMerge />
              {t("repository.changes")}
            </TabsTrigger>
            <TabsTrigger value="files" className="gap-1.5 px-3">
              <IconGitFork />
              {t("repository.files")}
            </TabsTrigger>
            <TabsTrigger value="branches" className="gap-1.5 px-3">
              <IconGitBranch />
              {t("repository.branches")}
            </TabsTrigger>
          </TabsList>
        </div>

        <TabsContent value="graph" className="min-h-0 overflow-hidden">
          <RepositoryGraph
            projectId={project.id}
            snapshot={repositoryQuery.data}
            snapshotError={repositoryQuery.error}
            branches={branches}
            projectTasks={projectTasks}
            selectedTask={selectedWorktreeTask}
            activeBranch={activeBranch}
            loading={repositoryQuery.isPending || branchesQuery.isFetching}
          />
        </TabsContent>
        <TabsContent value="changes" className="min-h-0 overflow-hidden">
          <RepositoryChanges
            projectId={project.id}
            task={selectedWorktreeTask}
            workingTree={repositoryQuery.data?.workingTree}
            syncOperation={repositoryQuery.data?.syncOperation}
          />
        </TabsContent>
        <TabsContent value="files" className="min-h-0 overflow-hidden">
          <ProjectFilesView
            key={`${project.id}:${repositoryQuery.data?.currentBranch ?? "HEAD"}`}
            projectId={project.id}
            rootPath={project.rootPath}
            branchName={repositoryQuery.data?.currentBranch ?? "HEAD"}
          />
        </TabsContent>
        <TabsContent value="branches" className="min-h-0 overflow-hidden">
          <RepositoryBranches
            projectId={project.id}
            branches={branches}
            projectTasks={projectTasks}
            currentBranch={repositoryQuery.data?.currentBranch}
            localBranchNames={repositoryQuery.data?.references
              .filter((reference) => reference.kind === "local")
              .map((reference) => reference.name) ?? []}
            loading={branchesQuery.isFetching}
            error={branchesQuery.error}
            onOpenChanges={() => setTab("changes")}
          />
        </TabsContent>
      </Tabs>
    </main>
  );
}

interface RepositoryHeaderProps {
  projectId?: string;
  projectName?: string;
  projectPath?: string;
  taskId?: string;
  activeBranch?: string;
  refreshing?: boolean;
  remoteStatus?: RepositoryRemoteStatus;
  syncOperation?: RepositorySyncOperation | null;
  onRefresh: () => void;
  onClose: () => void;
}

/** Keeps repository navigation in the existing app shell instead of opening a second window. */
function RepositoryHeader({
  projectId,
  projectName,
  projectPath,
  taskId,
  activeBranch,
  refreshing,
  remoteStatus,
  syncOperation,
  onRefresh,
  onClose,
}: RepositoryHeaderProps) {
  const { t } = useTranslation();

  return (
    <header className="flex h-14 shrink-0 items-center gap-2 border-b border-border px-3 sm:px-4">
      <Button
        type="button"
        variant="ghost"
        size="icon"
        aria-label={t("repository.backToChat")}
        title={t("repository.backToChat")}
        onClick={onClose}
      >
        <IconArrowLeft />
      </Button>
      <DragRegion>
        <div className="min-w-0">
          <p className="truncate text-sm font-semibold tracking-[-0.01em]">
            {projectName ?? t("repository.title")}
          </p>
          {projectPath && (
            <p className="truncate text-[11px] text-muted-foreground">{projectPath}</p>
          )}
        </div>
      </DragRegion>
      {activeBranch && (
        <Badge variant="secondary" className="ml-2 max-w-52 font-mono text-[11px]">
          <IconGitBranch />
          <span className="truncate">{activeBranch}</span>
        </Badge>
      )}
      {projectId && (
        <RepositorySyncControls
          projectId={projectId}
          activeBranch={activeBranch}
          remoteStatus={remoteStatus}
          syncOperation={syncOperation}
        />
      )}
      <div className="flex items-center gap-1">
        <Button
          type="button"
          variant="ghost"
          size="icon"
          disabled={refreshing}
          aria-label={t("repository.refresh")}
          title={t("repository.refresh")}
          onClick={onRefresh}
        >
          <IconRefresh />
        </Button>
        <LocationActionsButton taskId={taskId} projectPath={projectPath} />
        <WindowControls />
      </div>
    </header>
  );
}

interface RepositorySyncControlsProps {
  projectId: string;
  activeBranch?: string;
  remoteStatus?: RepositoryRemoteStatus;
  syncOperation?: RepositorySyncOperation | null;
}

/** Renders explicit fetch and push controls with a confirmation gate for network writes. */
function RepositorySyncControls({
  projectId,
  activeBranch,
  remoteStatus,
  syncOperation,
}: RepositorySyncControlsProps) {
  const { t } = useTranslation();
  const fetchRemote = useFetchRepository();
  const pullRepository = usePullRepository();
  const pushBranch = usePushRepositoryBranch();
  const resolveSync = useResolveRepositorySync();
  const [pushOpen, setPushOpen] = useState(false);
  const [strategyOpen, setStrategyOpen] = useState(false);
  const [diverged, setDiverged] = useState<{ ahead: number; behind: number } | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const pending = fetchRemote.isPending
    || pullRepository.isPending
    || pushBranch.isPending
    || resolveSync.isPending;
  const syncActive = syncOperation !== undefined && syncOperation !== null;

  const showPullOutcome = (outcome: PullRepositoryOutcome) => {
    if (outcome.kind === "diverged") {
      setDiverged({ ahead: outcome.ahead, behind: outcome.behind });
      setStrategyOpen(true);
      return;
    }
    if (outcome.kind === "conflicted") {
      setNotice(t("repository.pullConflict", {
        operation: outcome.operation === "merge"
          ? t("repository.syncOperationMerge")
          : t("repository.syncOperationRebase"),
      }));
      return;
    }
    setNotice(
      outcome.kind === "fastForwarded"
        ? t("repository.pullSucceeded")
        : outcome.kind === "alreadyUpToDate"
          ? t("repository.pullAlreadyUpToDate")
          : outcome.kind === "merged"
            ? t("repository.mergeSucceeded")
            : t("repository.rebaseSucceeded"),
    );
  };

  const handleFetch = async () => {
    setNotice(null);
    setError(null);
    try {
      await fetchRemote.mutateAsync({ projectId });
      setNotice(t("repository.fetchSucceeded"));
    } catch (fetchError) {
      setError(localizeContractError(fetchError, t));
    }
  };

  const handlePull = async () => {
    setNotice(null);
    setError(null);
    try {
      const response = await pullRepository.mutateAsync({
        projectId,
        strategy: "fastForwardOnly",
      });
      showPullOutcome(response.outcome);
    } catch (pullError) {
      setError(localizeContractError(pullError, t));
    }
  };

  const handleStrategy = async (strategy: "merge" | "rebase") => {
    setNotice(null);
    setError(null);
    try {
      const response = await pullRepository.mutateAsync({ projectId, strategy });
      setStrategyOpen(false);
      setDiverged(null);
      showPullOutcome(response.outcome);
    } catch (pullError) {
      setError(localizeContractError(pullError, t));
    }
  };

  const handleResolveSync = async (action: "continue" | "abort") => {
    setNotice(null);
    setError(null);
    try {
      const response = await resolveSync.mutateAsync({ projectId, action });
      setNotice(
        response.outcome === "completed"
          ? t("repository.syncContinued")
          : response.outcome === "aborted"
            ? t("repository.syncAborted")
            : t("repository.syncConflict"),
      );
    } catch (syncError) {
      setError(localizeContractError(syncError, t));
    }
  };

  const handlePush = async () => {
    setNotice(null);
    setError(null);
    try {
      await pushBranch.mutateAsync({ projectId });
      setPushOpen(false);
      setNotice(t("repository.pushSucceeded"));
    } catch (pushError) {
      setError(localizeContractError(pushError, t));
    }
  };

  return (
    <>
      <div className="ml-auto flex items-center gap-1">
        {remoteStatus?.ahead ? (
          <Badge variant="outline" className="text-[10px] text-emerald-600 dark:text-emerald-400">
            ↑ {remoteStatus.ahead}
          </Badge>
        ) : null}
        {remoteStatus?.behind ? (
          <Badge variant="outline" className="text-[10px] text-amber-600 dark:text-amber-400">
            ↓ {remoteStatus.behind}
          </Badge>
        ) : null}
        {syncActive ? (
          <Badge variant="destructive" className="max-w-32 truncate text-[10px]">
            {syncOperation === "merge"
              ? t("repository.syncOperationMerge")
              : t("repository.syncOperationRebase")}
          </Badge>
        ) : null}
        {syncActive ? (
          <>
            <Button
              type="button"
              variant="outline"
              size="sm"
              disabled={pending}
              title={t("repository.continueSync")}
              onClick={() => void handleResolveSync("continue")}
            >
              <span className="hidden xl:inline">
                {resolveSync.isPending ? t("repository.continuingSync") : t("repository.continueSync")}
              </span>
            </Button>
            <Button
              type="button"
              variant="destructive"
              size="sm"
              disabled={pending}
              title={t("repository.abortSync")}
              onClick={() => void handleResolveSync("abort")}
            >
              <span className="hidden xl:inline">
                {resolveSync.isPending ? t("repository.abortingSync") : t("repository.abortSync")}
              </span>
            </Button>
          </>
        ) : null}
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={pending || syncActive || activeBranch === undefined}
          title={t("repository.pull")}
          onClick={() => void handlePull()}
        >
          <IconArrowDown />
          <span className="hidden xl:inline">
            {pullRepository.isPending ? t("repository.pulling") : t("repository.pull")}
          </span>
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={pending || syncActive}
          title={t("repository.fetch")}
          onClick={() => void handleFetch()}
        >
          <IconDownload />
          <span className="hidden xl:inline">{fetchRemote.isPending ? t("repository.fetching") : t("repository.fetch")}</span>
        </Button>
        <Button
          type="button"
          variant="outline"
          size="sm"
          disabled={pending || syncActive || activeBranch === undefined}
          title={t("repository.push")}
          onClick={() => {
            setError(null);
            setPushOpen(true);
          }}
        >
          <IconUpload />
          <span className="hidden xl:inline">{pushBranch.isPending ? t("repository.pushing") : t("repository.push")}</span>
        </Button>
        {(notice !== null || error !== null) && (
          <span
            className={error === null ? "max-w-40 truncate text-[10px] text-emerald-600 dark:text-emerald-400" : "max-w-40 truncate text-[10px] text-destructive"}
            title={error ?? notice ?? undefined}
            role={error === null ? "status" : "alert"}
          >
            {error ?? notice}
          </span>
        )}
      </div>
      <AlertDialog
        open={strategyOpen}
        onOpenChange={(nextOpen) => !pending && setStrategyOpen(nextOpen)}
      >
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("repository.pullDivergedTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("repository.pullDivergedDescription", {
                ahead: diverged?.ahead ?? 0,
                behind: diverged?.behind ?? 0,
              })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {error !== null && <p className="text-xs text-destructive">{error}</p>}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending}>{t("common.cancel")}</AlertDialogCancel>
            <Button
              type="button"
              variant="outline"
              disabled={pending}
              onClick={() => void handleStrategy("merge")}
            >
              <IconGitMerge />
              {pullRepository.isPending ? t("repository.merging") : t("repository.merge")}
            </Button>
            <Button
              type="button"
              disabled={pending}
              onClick={() => void handleStrategy("rebase")}
            >
              <IconGitBranch />
              {pullRepository.isPending ? t("repository.rebasing") : t("repository.rebase")}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
      <AlertDialog open={pushOpen} onOpenChange={(nextOpen) => !pending && setPushOpen(nextOpen)}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>{t("repository.pushDialogTitle")}</AlertDialogTitle>
            <AlertDialogDescription>
              {t("repository.pushDialogDescription", { branch: activeBranch ?? "" })}
            </AlertDialogDescription>
          </AlertDialogHeader>
          {error !== null && <p className="text-xs text-destructive">{error}</p>}
          <AlertDialogFooter>
            <AlertDialogCancel disabled={pending}>{t("common.cancel")}</AlertDialogCancel>
            <AlertDialogAction disabled={pending} onClick={() => void handlePush()}>
              <IconUpload />
              {pushBranch.isPending ? t("repository.pushing") : t("repository.push")}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </>
  );
}

interface RepositoryBranchesProps {
  projectId: string;
  branches: ProjectBranch[];
  projectTasks: Task[];
  currentBranch?: string | null;
  localBranchNames: string[];
  loading: boolean;
  error: Error | null;
  onOpenChanges: () => void;
}

/** Lists repository refs and exposes safe create/checkout actions beside managed worktrees. */
function RepositoryBranches({
  projectId,
  branches,
  projectTasks,
  currentBranch,
  localBranchNames,
  loading,
  error,
  onOpenChanges,
}: RepositoryBranchesProps) {
  const { t } = useTranslation();
  const selectTask = useWorkspaceSelectionStore((state) => state.selectTask);
  const setRepositoryOpen = useUiStore((state) => state.setRepositoryOpen);
  const createBranch = useCreateRepositoryBranch();
  const checkoutBranch = useCheckoutRepositoryBranch();
  const [createDialogOpen, setCreateDialogOpen] = useState(false);
  const [actionNotice, setActionNotice] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const worktreeTasks = projectTasks.filter((task) => task.workspaceMode === "worktree");

  if (error) {
    return <RepositoryEmptyState title={t("repository.branchesError")} description={localizeContractError(error, t)} />;
  }

  const handleCreateBranch = async (values: Record<string, string>) => {
    const branchName = values.branchName?.trim() ?? "";
    await createBranch.mutateAsync({ projectId, branchName });
    setActionError(null);
    setActionNotice(t("repository.branchCreated", { branchName }));
  };

  const handleCheckout = async (branchName: string) => {
    setActionNotice(null);
    setActionError(null);
    try {
      await checkoutBranch.mutateAsync({ projectId, branchName });
      setActionNotice(t("repository.branchCheckedOut", { branchName }));
    } catch (checkoutError) {
      setActionError(localizeContractError(checkoutError, t));
    }
  };

  return (
    <ScrollArea className="h-full">
      <div className="mx-auto w-full max-w-4xl space-y-8 p-6 sm:p-8">
        <section>
          <div className="mb-3 flex items-center gap-2">
            <IconGitBranch className="size-4 text-muted-foreground" />
            <h2 className="text-sm font-semibold">{t("repository.localBranches")}</h2>
            <Badge variant="secondary">{loading ? "…" : branches.length}</Badge>
            <Button
              type="button"
              variant="outline"
              size="sm"
              className="ml-auto"
              onClick={() => {
                setActionError(null);
                setCreateDialogOpen(true);
              }}
            >
              <IconPlus />
              {t("repository.createBranch")}
            </Button>
          </div>
          {actionNotice && (
            <p className="mb-3 text-xs text-emerald-600 dark:text-emerald-400" role="status">
              {actionNotice}
            </p>
          )}
          {actionError && (
            <p className="mb-3 text-xs text-destructive" role="alert">
              {actionError}
            </p>
          )}
          <div className="overflow-hidden rounded-lg border border-border">
            {loading && <RefPlaceholder label={t("repository.loadingBranches")} className="p-4" />}
            {!loading && branches.length === 0 && <RefPlaceholder label={t("repository.noBranches")} className="p-4" />}
            {!loading && branches.map((branch) => (
              <div key={branch.refName} className="flex items-center gap-3 border-b border-border px-4 py-3 last:border-b-0">
                <IconGitBranch className="size-4 text-muted-foreground" />
                <span className="min-w-0 flex-1 truncate text-sm font-medium">{branch.displayName}</span>
                <span className="truncate font-mono text-xs text-muted-foreground">{branch.refName}</span>
                {localBranchNames.includes(branch.name) ? (
                  branch.name === currentBranch ? (
                    <Badge variant="secondary">{t("repository.currentBranch")}</Badge>
                  ) : (
                    <Button
                      type="button"
                      variant="ghost"
                      size="sm"
                      disabled={checkoutBranch.isPending}
                      onClick={() => void handleCheckout(branch.name)}
                    >
                      {checkoutBranch.isPending ? t("repository.checkingOut") : t("repository.checkout")}
                    </Button>
                  )
                ) : (
                  <Badge variant="outline">{t("repository.remoteBranch")}</Badge>
                )}
              </div>
            ))}
          </div>
        </section>
        <section>
          <div className="mb-3 flex items-center gap-2">
            <IconGitFork className="size-4 text-muted-foreground" />
            <h2 className="text-sm font-semibold">{t("repository.worktrees")}</h2>
            <Badge variant="secondary">{worktreeTasks.length}</Badge>
          </div>
          <div className="overflow-hidden rounded-lg border border-border">
            {worktreeTasks.length === 0 && <RefPlaceholder label={t("repository.noWorktrees")} className="p-4" />}
            {worktreeTasks.map((task) => (
              <button
                key={task.id}
                type="button"
                className="flex w-full items-center gap-3 border-b border-border px-4 py-3 text-left last:border-b-0 hover:bg-muted/50"
                onClick={() => {
                  selectTask(task.id, task.projectId);
                  setRepositoryOpen(true);
                  onOpenChanges();
                }}
              >
                <IconGitBranch className="size-4 text-sky-600" />
                <span className="min-w-0 flex-1 truncate text-sm font-medium">{task.title}</span>
                <span className="text-xs text-muted-foreground">{t("repository.openChanges")}</span>
              </button>
            ))}
          </div>
        </section>
      </div>
      <EntityDialog
        open={createDialogOpen}
        title={t("repository.createBranchTitle")}
        description={t("repository.createBranchDescription")}
        submitLabel={t("repository.createBranch")}
        fields={[{
          name: "branchName",
          kind: "text",
          label: t("repository.branchName"),
          value: "",
          placeholder: t("repository.branchNamePlaceholder"),
        }]}
        onOpenChange={setCreateDialogOpen}
        onSubmit={handleCreateBranch}
      />
    </ScrollArea>
  );
}

/** Keeps the existing task review surface as the Changes tab for managed worktrees. */
function RepositoryChanges({
  projectId,
  task,
  workingTree,
  syncOperation,
}: {
  projectId: string;
  task?: Task;
  workingTree?: RepositoryWorkingTree;
  syncOperation?: RepositorySyncOperation | null;
}) {
  const { t } = useTranslation();

  if (task === undefined) {
    return (
      <div className="flex h-full min-h-0 flex-col">
        {workingTree && workingTree.conflictedFiles > 0 ? (
          <div className="shrink-0 border-b border-destructive/30 bg-destructive/10 px-4 py-2 text-xs text-destructive">
            {t("repository.conflictedFiles", { count: workingTree.conflictedFiles })}
          </div>
        ) : null}
        <div className="min-h-0 flex-1">
          <RepositoryWorkingTreeDiffView
            projectId={projectId}
            workingTree={workingTree}
            syncOperation={syncOperation}
          />
        </div>
      </div>
    );
  }

  return (
    <TaskDiffView
      taskId={task.id}
      viewType="unified"
      fileTreeOpen
      onFileTreeOpenChange={() => undefined}
    />
  );
}

/** Renders a consistent empty state for missing repository context and staged slices. */
function RepositoryEmptyState({ title, description }: { title: string; description: string }) {
  return (
    <div className="flex min-h-0 flex-1 items-center justify-center p-6">
      <section className="max-w-md text-center">
        <div className="mx-auto mb-4 flex size-12 items-center justify-center rounded-xl border border-border bg-muted/50">
          <IconGitFork className="size-6 text-muted-foreground" />
        </div>
        <h1 className="text-base font-semibold">{title}</h1>
        <p className="mt-2 text-sm leading-6 text-muted-foreground">{description}</p>
      </section>
    </div>
  );
}

/** Shows loading and empty ref states without allowing the rail to jump in height. */
function RefPlaceholder({ label, className }: { label: string; className?: string }) {
  return <p className={`px-2 py-1 text-xs text-muted-foreground ${className ?? ""}`}>{label}</p>;
}

/** Selects the conventional base ref while preserving repositories with custom defaults. */
function preferredBranch(branches: ProjectBranch[]): string {
  return branches.find((branch) => branch.name === "main")?.displayName
    ?? branches.find((branch) => branch.name === "master")?.displayName
    ?? branches[0]?.displayName
    ?? "HEAD";
}
