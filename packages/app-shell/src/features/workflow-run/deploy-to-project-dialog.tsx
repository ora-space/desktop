import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { TFunction } from "i18next";
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
  IconRocket,
  IconRoute,
} from "@tabler/icons-react";
import { useProjects } from "../../state/hooks/use-projects";
import { useWorkspaces } from "../../state/hooks/use-workspaces";
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
   * is a create form for the selected project's main Workspace.
   */
  initialProjectId?: string | null;
  onOpenChange: (open: boolean) => void;
}

/**
 * Deploy semantics (product contract):
 * - Deploy creates one pending run against the project's main Workspace.
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
  const workspacesQuery = useWorkspaces();
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
  const [projectPickerOpen, setProjectPickerOpen] = useState(false);
  const [attemptedSubmit, setAttemptedSubmit] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const selectedProject = projects.find((project) => project.id === projectId);
  const selectedWorkspace = workspacesQuery.data?.find(
    (workspace) =>
      workspace.projectId === projectId && workspace.kind === "main",
  );

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
    const workspace = selectedWorkspace;
    if (
      workflow === null ||
      projectMissing ||
      nameMissing ||
      workspace === undefined
    ) {
      setAttemptedSubmit(true);
      return;
    }
    setError(null);
    try {
      const result = await createRun.mutateAsync({
        projectId,
        workspaceId: workspace.id,
        workflowId: workflow.id,
        name: resolvedRunName,
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
    setProjectPickerOpen(false);
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

/** Maps the persisted-backend deploy failures onto their translated contract messages. */
function resolveDeployError(cause: unknown, t: TFunction): string {
  return localizeContractError(cause, t);
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
