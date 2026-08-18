import {
  startTransition,
  useCallback,
  useDeferredValue,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { useTranslation } from "react-i18next";
import type { ProjectBranch } from "@ora/contracts";
import type { TFunction } from "i18next";
import { useVirtualizer } from "@tanstack/react-virtual";
import { localizeContractError } from "../../i18n/contract-error";
import type { WorkflowDefinitionInput } from "@ora/workflow-runtime";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Button,
  Command,
  CommandEmpty,
  CommandGroup,
  CommandInput,
  CommandItem,
  CommandList,
  Input,
  Popover,
  PopoverContent,
  PopoverTrigger,
  Spinner,
  cn,
} from "@ora/ui";
import {
  IconChevronDown,
  IconFolder,
  IconGitBranch,
  IconRefresh,
  IconRocket,
  IconRoute,
  IconSearch,
} from "@tabler/icons-react";
import { useProjectBranches } from "../../state/hooks/use-project-branches";
import { useProjects } from "../../state/hooks/use-projects";
import {
  useCreateWorkflowRun,
  useWorkflowRunsByWorkflow,
} from "../../state/hooks/use-workflow-runs";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";

interface DeployToProjectDialogProps {
  open: boolean;
  workflow: { id: string; name: string } | null;
  /**
   * Project already chosen by the caller (sidebar create). When set, the dialog
   * is a create form: name and base branch only, matching worktree-task create.
   */
  initialProjectId?: string | null;
  onOpenChange: (open: boolean) => void;
}

const BRANCH_ROW_HEIGHT = 32;
const BRANCH_LIST_MAX_HEIGHT = 256;

const MENU_ITEM_CLASS =
  "flex w-full cursor-default items-center gap-1.5 rounded-sm px-2 py-1.5 text-left text-sm text-foreground outline-none hover:bg-muted focus-visible:bg-muted";

/**
 * Deploy semantics (product contract):
 * - Deploy creates one pending run against the workflow's published snapshot under the
 *   chosen project; the run-task owns the project association (no mount concept).
 * - Opening deploy from settings auto-publishes the current draft when no published
 *   snapshot exists yet, so the form is never filled only to hit that precondition.
 * - The run name is required at creation time; the backend falls back to a generated
 *   title only when the contract call omits it.
 * - Projects that already have runs of this workflow are grouped first as a reverse view.
 */
export function DeployToProjectDialog({
  open,
  workflow,
  initialProjectId = null,
  onOpenChange,
}: DeployToProjectDialogProps) {
  const { t } = useTranslation();
  const projectLocked = Boolean(initialProjectId);
  const projectsQuery = useProjects();
  const projects = useMemo(
    () => projectsQuery.data ?? [],
    [projectsQuery.data],
  );
  const runsQuery = useWorkflowRunsByWorkflow(
    open && !projectLocked ? (workflow?.id ?? null) : null,
  );
  const deployedProjectIds = useMemo(
    () => new Set((runsQuery.data ?? []).map((run) => run.projectId)),
    [runsQuery.data],
  );
  const createRun = useCreateWorkflowRun();
  const selectWorkflowRun = useWorkspaceSelectionStore(
    (s) => s.selectWorkflowRun,
  );
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const [projectId, setProjectId] = useState<string>("");
  const [name, setName] = useState<string>("");
  const [baseBranch, setBaseBranch] = useState<string>("");
  const [projectPickerOpen, setProjectPickerOpen] = useState(false);
  const [branchPickerOpen, setBranchPickerOpen] = useState(false);
  const [attemptedSubmit, setAttemptedSubmit] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const branchesQuery = useProjectBranches(projectId || null);
  const projectBranches = branchesQuery.data ?? [];
  // First fetch has no cache yet. Keep the trigger clickable and animate inside the
  // panel so a slow git fetch never freezes the first open interaction.
  const branchesLoading = projectId !== "" && branchesQuery.isPending;
  const branchesRefreshing =
    projectId !== "" && branchesQuery.isFetching && !branchesQuery.isPending;

  const selectedProject = projects.find((project) => project.id === projectId);
  const selectedBranch = projectBranches.find(
    (branch) =>
      branch.refName ===
      (baseBranch === "" ? preferredBaseBranch(projectBranches) : baseBranch),
  );

  // Derive the default branch during render: an untouched choice falls back to the
  // project's conventional primary branch, and switching projects resets the choice.
  const effectiveBaseBranch =
    baseBranch === "" ? preferredBaseBranch(projectBranches) : baseBranch;

  const deployedProjects = useMemo(
    () => projects.filter((project) => deployedProjectIds.has(project.id)),
    [deployedProjectIds, projects],
  );
  const otherProjects = useMemo(
    () => projects.filter((project) => !deployedProjectIds.has(project.id)),
    [deployedProjectIds, projects],
  );

  const busy = createRun.isPending;
  // Prefer the typed value; fall back to the workflow title so an empty field still deploys.
  const resolvedRunName = name.trim() || (workflow?.name.trim() ?? "");
  const nameMissing = resolvedRunName === "";
  const projectMissing = projectId === "";
  const branchMissing =
    !projectMissing && !branchesLoading && effectiveBaseBranch === "";

  // Seed the run name when the dialog opens or the target workflow changes (render-phase
  // reset avoids an effect-driven cascading setState on open).
  const [nameSeedKey, setNameSeedKey] = useState<string | null>(null);
  const nextNameSeedKey =
    open && workflow !== null ? `${workflow.id}:${workflow.name}` : null;
  if (
    nextNameSeedKey !== null &&
    nextNameSeedKey !== nameSeedKey &&
    workflow !== null
  ) {
    setNameSeedKey(nextNameSeedKey);
    setName(workflow.name);
    if (initialProjectId) {
      setProjectId(initialProjectId);
    }
  }
  if (!open && nameSeedKey !== null) {
    setNameSeedKey(null);
  }

  /** Creates a pending run under the chosen project and focuses it in the shell. */
  async function submit(): Promise<void> {
    if (
      workflow === null ||
      projectMissing ||
      nameMissing ||
      branchMissing ||
      branchesLoading
    ) {
      setAttemptedSubmit(true);
      return;
    }
    setError(null);
    try {
      const result = await createRun.mutateAsync({
        projectId,
        workflowId: workflow.id,
        name: resolvedRunName,
        baseBranch:
          effectiveBaseBranch === "" ? undefined : effectiveBaseBranch,
      });
      useUiStore.setState((state) => ({
        expandedProjects: new Set([...state.expandedProjects, projectId]),
      }));
      selectWorkflowRun(result.run.id, projectId);
      onOpenChange(false);
      resetLocalState();
      setSettingsOpen(false);
    } catch (cause) {
      setError(resolveDeployError(cause, t));
    }
  }

  function resetLocalState(): void {
    setError(null);
    setProjectId("");
    setName("");
    setBaseBranch("");
    setProjectPickerOpen(false);
    setBranchPickerOpen(false);
    setAttemptedSubmit(false);
  }

  return (
    <AlertDialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          resetLocalState();
        }
        onOpenChange(next);
      }}
    >
      <AlertDialogContent className="sm:max-w-md">
        <AlertDialogHeader>
          <AlertDialogTitle>
            {projectLocked
              ? t("sidebar.newWorkflow")
              : t("workflowRun.deployTitle")}
          </AlertDialogTitle>
          {workflow === null ? (
            <AlertDialogDescription>
              {t("workflowRun.deployPickWorkflow")}
            </AlertDialogDescription>
          ) : projectLocked ? (
            <AlertDialogDescription className="sr-only">
              {t("sidebar.newWorkflow")}
            </AlertDialogDescription>
          ) : (
            <AlertDialogDescription className="sr-only">
              {t("workflowRun.deployDescription", { name: workflow.name })}
            </AlertDialogDescription>
          )}
        </AlertDialogHeader>

        <div className="mt-2 space-y-3">
          <div className="space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">
              {t("workflowRun.deployRunName")}
            </p>
            <Input
              value={name}
              onChange={(event) => setName(event.target.value)}
              aria-label={t("workflowRun.deployRunName")}
              aria-invalid={attemptedSubmit && nameMissing}
              placeholder={
                workflow === null
                  ? t("workflowRun.deployRunNamePlaceholder")
                  : t("workflowRun.deployRunNamePlaceholderWithDefault", {
                      name: workflow.name,
                    })
              }
              disabled={workflow === null}
              onKeyDown={(event) => {
                if (event.key === "Enter") {
                  event.preventDefault();
                  void submit();
                }
              }}
            />
            {attemptedSubmit && nameMissing ? (
              <p
                className="text-[11px] leading-5 text-destructive"
                role="status"
              >
                {t("workflowRun.deployRequiredRunName")}
              </p>
            ) : null}
          </div>

          {!projectLocked ? (
            <div className="space-y-1.5">
              <p className="text-xs font-medium text-muted-foreground">
                {t("workflowRun.deployProject")}
              </p>
              <Popover
                open={projectPickerOpen}
                onOpenChange={setProjectPickerOpen}
              >
                <PopoverTrigger
                  render={
                    <Button
                      type="button"
                      variant="outline"
                      className={cn(
                        "h-9 w-full justify-between px-3 font-normal",
                        attemptedSubmit &&
                          projectMissing &&
                          "border-destructive",
                      )}
                      disabled={projects.length === 0}
                      aria-label={t("workflowRun.deployProject")}
                      aria-invalid={attemptedSubmit && projectMissing}
                    />
                  }
                >
                  <span className="flex min-w-0 items-center gap-2">
                    <IconFolder className="size-3.5 shrink-0 text-muted-foreground" />
                    <span
                      className={cn(
                        "truncate",
                        selectedProject
                          ? "text-foreground"
                          : "text-muted-foreground",
                      )}
                    >
                      {selectedProject?.name ??
                        t("workflowRun.deployProjectEmpty")}
                    </span>
                  </span>
                  <IconChevronDown className="size-3.5 shrink-0 opacity-50" />
                </PopoverTrigger>
                <PopoverContent align="start" className="w-80 p-0">
                  <Command>
                    <CommandInput
                      placeholder={t("workflowRun.deployProjectSearch")}
                      className="text-sm"
                    />
                    <CommandList className="max-h-64">
                      <CommandEmpty className="py-6 text-sm">
                        {t("workflowRun.deployProjectEmptySearch")}
                      </CommandEmpty>
                      {deployedProjects.length > 0 && (
                        <CommandGroup
                          heading={t("workflowRun.deployGroupHasRuns")}
                        >
                          {deployedProjects.map((project) => (
                            <CommandItem
                              key={project.id}
                              value={project.name}
                              data-checked={project.id === projectId}
                              className="gap-1.5 rounded-sm px-2 py-1.5 text-sm text-foreground focus:bg-muted focus:text-foreground"
                              onSelect={() => {
                                setProjectId(project.id);
                                setBaseBranch("");
                                setProjectPickerOpen(false);
                              }}
                            >
                              <IconRoute className="size-3.5 text-muted-foreground" />
                              <span className="min-w-0 flex-1 truncate">
                                {project.name}
                              </span>
                            </CommandItem>
                          ))}
                        </CommandGroup>
                      )}
                      {otherProjects.length > 0 && (
                        <CommandGroup
                          heading={
                            deployedProjects.length > 0
                              ? t("workflowRun.deployGroupOther")
                              : undefined
                          }
                        >
                          {otherProjects.map((project) => (
                            <CommandItem
                              key={project.id}
                              value={project.name}
                              data-checked={project.id === projectId}
                              className="gap-1.5 rounded-sm px-2 py-1.5 text-sm text-foreground focus:bg-muted focus:text-foreground"
                              onSelect={() => {
                                setProjectId(project.id);
                                setBaseBranch("");
                                setProjectPickerOpen(false);
                              }}
                            >
                              <IconFolder className="size-3.5 text-muted-foreground" />
                              <span className="min-w-0 flex-1 truncate">
                                {project.name}
                              </span>
                            </CommandItem>
                          ))}
                        </CommandGroup>
                      )}
                    </CommandList>
                  </Command>
                </PopoverContent>
              </Popover>
              {attemptedSubmit && projectMissing ? (
                <p
                  className="text-[11px] leading-5 text-destructive"
                  role="status"
                >
                  {t("workflowRun.deployRequiredProject")}
                </p>
              ) : projectId !== "" ? (
                <p className="text-[11px] leading-5 text-muted-foreground">
                  {t("workflowRun.deployHintDeploy")}
                </p>
              ) : null}
            </div>
          ) : null}

          <div className="space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">
              {t("workflowRun.deployBaseBranch")}
            </p>
            <Popover
              open={branchPickerOpen}
              onOpenChange={(next) => {
                if (projectMissing) {
                  return;
                }
                // Open in a transition so the click paints immediately; the branch
                // list mounts as non-urgent work. Always kick a background refetch so
                // a warm cache cannot hide branches that landed after the last fetch.
                if (next) {
                  startTransition(() => setBranchPickerOpen(true));
                  void branchesQuery.refetch();
                  return;
                }
                setBranchPickerOpen(false);
              }}
            >
              <PopoverTrigger
                render={
                  <Button
                    type="button"
                    variant="outline"
                    className={cn(
                      "h-9 w-full justify-between px-3 font-normal",
                      attemptedSubmit && branchMissing && "border-destructive",
                    )}
                    disabled={projectMissing}
                    aria-label={t("workflowRun.deployBaseBranch")}
                    aria-busy={branchesLoading}
                    aria-invalid={attemptedSubmit && branchMissing}
                  />
                }
              >
                <span className="flex min-w-0 items-center gap-2">
                  {branchesLoading ? (
                    <Spinner className="size-3.5 shrink-0 text-muted-foreground" />
                  ) : (
                    <IconGitBranch className="size-3.5 shrink-0 text-muted-foreground" />
                  )}
                  <span
                    className={cn(
                      "truncate",
                      selectedBranch || branchesLoading
                        ? "text-foreground"
                        : "text-muted-foreground",
                    )}
                  >
                    {branchesLoading
                      ? t("workflowRun.deployBaseBranchLoading")
                      : (selectedBranch?.displayName ??
                        t("workflowRun.deployBaseBranchEmpty"))}
                  </span>
                </span>
                <span className="flex shrink-0 items-center gap-1">
                  {branchesRefreshing ? (
                    <Spinner className="size-3 opacity-60" />
                  ) : null}
                  <IconChevronDown className="size-3.5 opacity-50" />
                </span>
              </PopoverTrigger>
              <PopoverContent align="start" className="w-80 gap-0 p-0">
                <DeployBranchPicker
                  branches={projectBranches}
                  loading={branchesLoading}
                  refreshing={branchesRefreshing}
                  selectedRefName={effectiveBaseBranch}
                  onRefresh={() => {
                    void branchesQuery.refetch();
                  }}
                  onSelect={(refName) => {
                    setBaseBranch(refName);
                    setBranchPickerOpen(false);
                  }}
                />
              </PopoverContent>
            </Popover>
            {branchesLoading && !projectLocked ? (
              <p className="flex items-center gap-1.5 text-[11px] leading-5 text-muted-foreground">
                <Spinner className="size-3 shrink-0" />
                {t("workflowRun.deployBaseBranchLoadingHint")}
              </p>
            ) : attemptedSubmit && branchMissing ? (
              <p
                className="text-[11px] leading-5 text-destructive"
                role="status"
              >
                {t("workflowRun.deployRequiredBaseBranch")}
              </p>
            ) : !projectMissing &&
              projectBranches.length === 0 &&
              !branchesQuery.isPending ? (
              <p
                className="text-[11px] leading-5 text-destructive"
                role="status"
              >
                {t("workflowRun.deployBaseBranchUnavailable")}
              </p>
            ) : null}
          </div>
        </div>

        {error && (
          <p className="mt-2 text-xs text-destructive" role="alert">
            {error}
          </p>
        )}
        <AlertDialogFooter>
          <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
          <Button
            type="button"
            disabled={busy || workflow === null}
            onClick={() => void submit()}
          >
            {busy ? (
              <span className="inline-flex items-center gap-1.5">
                <Spinner className="size-3.5" />
                {projectLocked
                  ? t("common.creating")
                  : t("workflowRun.deploying")}
              </span>
            ) : projectLocked ? (
              t("dialog.createTask")
            ) : (
              t("workflowRun.deployConfirm")
            )}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/**
 * Virtualized searchable branch list.
 * Renders only visible rows so large repos stay scrollable without mounting
 * hundreds of DOM nodes on open. A background refetch keeps the cache honest.
 */
function DeployBranchPicker({
  branches,
  loading,
  refreshing,
  selectedRefName,
  onRefresh,
  onSelect,
}: {
  branches: ProjectBranch[];
  loading: boolean;
  refreshing: boolean;
  selectedRefName: string;
  onRefresh: () => void;
  onSelect: (refName: string) => void;
}) {
  const { t } = useTranslation();
  const [query, setQuery] = useState("");
  const [listReady, setListReady] = useState(false);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const deferredQuery = useDeferredValue(query);
  const filteredBranches = useMemo(() => {
    const needle = deferredQuery.trim().toLowerCase();
    if (needle === "") {
      return branches;
    }
    return branches.filter(
      (branch) =>
        branch.displayName.toLowerCase().includes(needle) ||
        branch.name.toLowerCase().includes(needle) ||
        branch.refName.toLowerCase().includes(needle),
    );
  }, [branches, deferredQuery]);
  const getItemKey = useCallback(
    (index: number) => filteredBranches[index]?.refName ?? index,
    [filteredBranches],
  );
  // TanStack Virtual owns mutable scroll metrics outside React memoization.
  // eslint-disable-next-line react-hooks/incompatible-library
  const virtualizer = useVirtualizer({
    count: filteredBranches.length,
    getScrollElement: () => listRef.current,
    estimateSize: () => BRANCH_ROW_HEIGHT,
    getItemKey,
    overscan: 12,
    initialRect: { width: 320, height: BRANCH_LIST_MAX_HEIGHT },
    enabled: !loading && filteredBranches.length > 0,
  });

  useEffect(() => {
    searchInputRef.current?.focus();
  }, []);

  // Reveal the virtual list one frame after mount so the popover chrome paints first.
  useEffect(() => {
    if (loading) {
      setListReady(false);
      return;
    }
    const frame = requestAnimationFrame(() => setListReady(true));
    return () => cancelAnimationFrame(frame);
  }, [loading, filteredBranches.length]);

  if (loading) {
    return (
      <div className="flex flex-col items-center justify-center gap-2 px-3 py-10 text-sm text-muted-foreground">
        <Spinner className="size-4" />
        {t("workflowRun.deployBaseBranchLoading")}
      </div>
    );
  }

  return (
    <div className="flex min-h-0 flex-col">
      <div className="flex items-center gap-1 border-b border-border px-2 py-1.5">
        <IconSearch className="ml-0.5 size-3.5 shrink-0 text-muted-foreground" />
        <input
          ref={searchInputRef}
          value={query}
          onChange={(event) => setQuery(event.target.value)}
          placeholder={t("workflowRun.deployBaseBranchSearch")}
          aria-label={t("workflowRun.deployBaseBranchSearch")}
          className="h-7 min-w-0 flex-1 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        />
        <Button
          type="button"
          variant="ghost"
          size="sm"
          className="size-7 shrink-0 px-0"
          disabled={refreshing}
          aria-label={t("workflowRun.deployBaseBranchRefresh")}
          title={t("workflowRun.deployBaseBranchRefresh")}
          onClick={onRefresh}
        >
          {refreshing ? (
            <Spinner className="size-3.5" />
          ) : (
            <IconRefresh className="size-3.5" />
          )}
        </Button>
      </div>
      {refreshing ? (
        <div className="flex items-center gap-1.5 border-b border-border bg-muted/40 px-2.5 py-1 text-[11px] text-muted-foreground">
          <Spinner className="size-3 shrink-0" />
          {t("workflowRun.deployBaseBranchRefreshing")}
        </div>
      ) : null}
      <div
        ref={listRef}
        className={cn(
          "max-h-64 overflow-y-auto overscroll-contain p-1 transition-opacity duration-150",
          listReady ? "opacity-100" : "opacity-0",
        )}
        style={{ maxHeight: BRANCH_LIST_MAX_HEIGHT }}
        role="listbox"
      >
        {filteredBranches.length === 0 ? (
          <p className="px-2 py-6 text-center text-sm text-muted-foreground">
            {t("workflowRun.deployBaseBranchEmptySearch")}
          </p>
        ) : (
          <div
            className="relative w-full"
            style={{ height: virtualizer.getTotalSize() }}
          >
            {virtualizer.getVirtualItems().map((item) => {
              const branch = filteredBranches[item.index];
              if (branch === undefined) {
                return null;
              }
              const selected = branch.refName === selectedRefName;
              return (
                <button
                  key={branch.refName}
                  type="button"
                  role="option"
                  aria-selected={selected}
                  className={cn(
                    MENU_ITEM_CLASS,
                    "absolute top-0 left-0 w-full",
                    selected && "bg-muted",
                  )}
                  style={{
                    height: item.size,
                    transform: `translateY(${item.start}px)`,
                  }}
                  onClick={() => onSelect(branch.refName)}
                >
                  <IconGitBranch className="size-3.5 shrink-0 text-muted-foreground" />
                  <span className="min-w-0 flex-1 truncate">
                    {branch.displayName}
                  </span>
                </button>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
}

/** Maps the persisted-backend deploy failures onto their translated contract messages. */
function resolveDeployError(cause: unknown, t: TFunction): string {
  return localizeContractError(cause, t);
}

/** Prefers a fetched conventional primary branch while preserving repositories with custom defaults. */
function preferredBaseBranch(branches: ProjectBranch[]): string {
  return (
    branches.find((branch) => branch.name === "main")?.refName ??
    branches.find((branch) => branch.name === "master")?.refName ??
    branches[0]?.refName ??
    ""
  );
}

interface DeployWorkflowButtonProps {
  workflow: WorkflowDefinitionInput | null;
  /**
   * Runs before the deploy dialog opens (flush draft, auto-publish when needed).
   * Return false to abort opening the dialog.
   */
  onPrepareDeploy?: () => Promise<boolean>;
}

/** Toolbar control that opens DeployToProjectDialog for the active library workflow. */
export function DeployWorkflowButton({
  workflow,
  onPrepareDeploy,
}: DeployWorkflowButtonProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const [preparing, setPreparing] = useState(false);

  /** Ensures the workflow is deployable, then opens the project/run dialog. */
  async function handleClick(): Promise<void> {
    if (workflow === null || preparing) {
      return;
    }
    setPreparing(true);
    try {
      if (onPrepareDeploy !== undefined) {
        const ready = await onPrepareDeploy();
        if (!ready) {
          return;
        }
      }
      setOpen(true);
    } finally {
      setPreparing(false);
    }
  }

  return (
    <>
      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={workflow === null || preparing}
        onClick={() => void handleClick()}
      >
        {preparing ? <Spinner className="size-3.5" /> : <IconRocket />}
        {t("workflowRun.deployAction")}
      </Button>
      <DeployToProjectDialog
        open={open}
        workflow={workflow}
        onOpenChange={setOpen}
      />
    </>
  );
}
