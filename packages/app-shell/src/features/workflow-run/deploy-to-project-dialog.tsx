import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ProjectBranch } from "@ora/contracts";
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
  NativeSelect,
  NativeSelectOption,
  Popover,
  PopoverContent,
  PopoverTrigger,
  cn,
} from "@ora/ui";
import {
  IconChevronDown,
  IconFolder,
  IconRoute,
} from "@tabler/icons-react";
import { useProjectBranches } from "../../state/hooks/use-project-branches";
import { useProjects } from "../../state/hooks/use-projects";
import { useCreateWorkflowRun, useWorkflowRunsByWorkflow } from "../../state/hooks/use-workflow-runs";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";

interface DeployToProjectDialogProps {
  open: boolean;
  workflow: WorkflowDefinitionInput | null;
  onOpenChange: (open: boolean) => void;
}

const MENU_ITEM_CLASS =
  "gap-1.5 rounded-sm px-2 py-1.5 text-sm text-foreground focus:bg-muted focus:text-foreground";

/**
 * Deploy semantics (product contract):
 * - Deploy creates one pending run against the workflow's published snapshot under the
 *   chosen project; the run-task owns the project association (no mount concept).
 * - The run name is required at creation time; the backend falls back to a generated
 *   title only when the contract call omits it.
 * - Projects that already have runs of this workflow are grouped first as a reverse view.
 */
export function DeployToProjectDialog({
  open,
  workflow,
  onOpenChange,
}: DeployToProjectDialogProps) {
  const { t } = useTranslation();
  const projectsQuery = useProjects();
  const projects = useMemo(() => projectsQuery.data ?? [], [projectsQuery.data]);
  const runsQuery = useWorkflowRunsByWorkflow(open ? workflow?.id : null);
  const deployedProjectIds = useMemo(
    () => new Set((runsQuery.data ?? []).map((run) => run.projectId)),
    [runsQuery.data],
  );
  const createRun = useCreateWorkflowRun();
  const selectWorkflowRun = useWorkspaceSelectionStore((s) => s.selectWorkflowRun);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const [projectId, setProjectId] = useState<string>("");
  const [name, setName] = useState<string>("");
  const [baseBranch, setBaseBranch] = useState<string>("");
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const { data: projectBranches = [] } = useProjectBranches(projectId || null);

  const selectedProject = projects.find((project) => project.id === projectId);

  // Derive the default branch during render: an untouched choice falls back to the
  // project's conventional primary branch, and switching projects resets the choice.
  const effectiveBaseBranch = baseBranch === ""
    ? preferredBaseBranch(projectBranches)
    : baseBranch;

  const deployedProjects = useMemo(
    () => projects.filter((project) => deployedProjectIds.has(project.id)),
    [deployedProjectIds, projects],
  );
  const otherProjects = useMemo(
    () => projects.filter((project) => !deployedProjectIds.has(project.id)),
    [deployedProjectIds, projects],
  );

  const busy = createRun.isPending;
  const nameMissing = name.trim() === "";
  const canSubmit = workflow !== null && projectId !== "" && !nameMissing && !busy;

  /** Creates a pending run under the chosen project and focuses it in the shell. */
  async function submit(): Promise<void> {
    if (workflow === null || projectId === "" || nameMissing) {
      return;
    }
    setError(null);
    try {
      const result = await createRun.mutateAsync({
        projectId,
        workflowId: workflow.id,
        name: name.trim(),
        baseBranch: effectiveBaseBranch === "" ? undefined : effectiveBaseBranch,
      });
      useUiStore.setState((state) => ({
        expandedProjects: new Set([...state.expandedProjects, projectId]),
      }));
      selectWorkflowRun(result.run.id, projectId);
      onOpenChange(false);
      setProjectId("");
      setName("");
      setSettingsOpen(false);
    } catch (cause) {
      setError(resolveDeployError(cause, t));
    }
  }

  function resetLocalState(): void {
    setError(null);
    setProjectId("");
    setName("");
    setPickerOpen(false);
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
          <AlertDialogTitle>{t("workflowRun.deployTitle")}</AlertDialogTitle>
          {workflow === null
            ? (
              <AlertDialogDescription>
                {t("workflowRun.deployPickWorkflow")}
              </AlertDialogDescription>
            )
            : (
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
              placeholder={t("workflowRun.deployRunNamePlaceholder")}
              disabled={workflow === null}
              onKeyDown={(event) => {
                if (event.key === "Enter" && canSubmit) {
                  event.preventDefault();
                  void submit();
                }
              }}
            />
          </div>

          <div className="space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">
              {t("workflowRun.deployProject")}
            </p>
            <Popover open={pickerOpen} onOpenChange={setPickerOpen}>
              <PopoverTrigger
                render={
                  <Button
                    type="button"
                    variant="outline"
                    className="h-9 w-full justify-between px-3 font-normal"
                    disabled={projects.length === 0}
                    aria-label={t("workflowRun.deployProject")}
                  />
                }
              >
                <span className="flex min-w-0 items-center gap-2">
                  <IconFolder className="size-3.5 shrink-0 text-muted-foreground" />
                  <span
                    className={cn(
                      "truncate",
                      selectedProject ? "text-foreground" : "text-muted-foreground",
                    )}
                  >
                    {selectedProject?.name ?? t("workflowRun.deployProjectEmpty")}
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
                      <CommandGroup heading={t("workflowRun.deployGroupHasRuns")}>
                        {deployedProjects.map((project) => (
                          <CommandItem
                            key={project.id}
                            value={project.name}
                            data-checked={project.id === projectId}
                            className={MENU_ITEM_CLASS}
                            onSelect={() => {
                              setProjectId(project.id);
                              setBaseBranch("");
                              setPickerOpen(false);
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
                            className={MENU_ITEM_CLASS}
                            onSelect={() => {
                              setProjectId(project.id);
                              setBaseBranch("");
                              setPickerOpen(false);
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
            {projectId !== "" && (
              <p className="text-[11px] leading-5 text-muted-foreground">
                {t("workflowRun.deployHintDeploy")}
              </p>
            )}
          </div>

          <div className="space-y-1.5">
            <p className="text-xs font-medium text-muted-foreground">
              {t("workflowRun.deployBaseBranch")}
            </p>
            <NativeSelect
              value={effectiveBaseBranch}
              disabled={projectId === "" || projectBranches.length === 0}
              aria-label={t("workflowRun.deployBaseBranch")}
              className="w-full"
              onChange={(event) => setBaseBranch(event.target.value)}
            >
              {projectBranches.map((branch) => (
                <NativeSelectOption key={branch.refName} value={branch.refName}>
                  {branch.displayName}
                </NativeSelectOption>
              ))}
            </NativeSelect>
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
            disabled={!canSubmit}
            onClick={() => void submit()}
          >
            {busy
              ? t("workflowRun.deploying")
              : t("workflowRun.deployConfirm")}
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

/** Prefers a fetched conventional primary branch while preserving repositories with custom defaults. */
function preferredBaseBranch(branches: ProjectBranch[]): string {
  return branches.find((branch) => branch.name === "main")?.refName
    ?? branches.find((branch) => branch.name === "master")?.refName
    ?? branches[0]?.refName
    ?? "";
}

interface DeployWorkflowButtonProps {
  workflow: WorkflowDefinitionInput | null;
}

/** Toolbar control that opens DeployToProjectDialog for the active library workflow. */
export function DeployWorkflowButton({ workflow }: DeployWorkflowButtonProps) {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  return (
    <>
      <Button
        type="button"
        variant="outline"
        size="sm"
        disabled={workflow === null}
        onClick={() => setOpen(true)}
      >
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
