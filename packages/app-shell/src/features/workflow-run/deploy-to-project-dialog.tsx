import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
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
import { useProjects } from "../../state/hooks/use-projects";
import {
  useCreateGraphWorkflowRun,
  useMountWorkflow,
} from "../../state/hooks/use-graph-workflow-runs";
import { useWorkflowMountsByDefinition } from "../../state/hooks/use-workflow-mounts";
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
 * - Mount is 1:1 per (project, definition). Remount refreshes the definition blob.
 * - Every confirm creates a new pending GraphWorkflowRun under that project
 *   (execution starts from the run workspace Start control).
 * - First deploy = mount + first run; later deploys = refresh mount + another run.
 */
export function DeployToProjectDialog({
  open,
  workflow,
  onOpenChange,
}: DeployToProjectDialogProps) {
  const { t } = useTranslation();
  const projectsQuery = useProjects();
  const projects = useMemo(() => projectsQuery.data ?? [], [projectsQuery.data]);
  const mountsQuery = useWorkflowMountsByDefinition(
    open ? workflow?.id : null,
  );
  const mountedProjectIds = useMemo(
    () => new Set((mountsQuery.data ?? []).map((mount) => mount.projectId)),
    [mountsQuery.data],
  );
  const mountWorkflow = useMountWorkflow();
  const createRun = useCreateGraphWorkflowRun();
  const selectWorkflowRun = useWorkspaceSelectionStore((s) => s.selectWorkflowRun);
  const setSettingsOpen = useUiStore((s) => s.setSettingsOpen);
  const [projectId, setProjectId] = useState<string>("");
  const [pickerOpen, setPickerOpen] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const selectedProject = projects.find((project) => project.id === projectId);
  const alreadyMounted =
    projectId !== "" && mountedProjectIds.has(projectId);

  const mountedProjects = useMemo(
    () => projects.filter((project) => mountedProjectIds.has(project.id)),
    [mountedProjectIds, projects],
  );
  const otherProjects = useMemo(
    () => projects.filter((project) => !mountedProjectIds.has(project.id)),
    [mountedProjectIds, projects],
  );

  const busy = mountWorkflow.isPending || createRun.isPending;
  const canSubmit = workflow !== null && projectId !== "" && !busy;

  /** Upserts mount, always creates a new run, then focuses that run in the shell. */
  async function submit(): Promise<void> {
    if (workflow === null || projectId === "") {
      return;
    }
    setError(null);
    try {
      await mountWorkflow.mutateAsync({ projectId, definition: workflow });
      const run = await createRun.mutateAsync({
        projectId,
        definitionId: workflow.id,
      });
      useUiStore.setState((state) => ({
        expandedProjects: new Set([...state.expandedProjects, projectId]),
      }));
      selectWorkflowRun(run.id, projectId);
      onOpenChange(false);
      setProjectId("");
      setSettingsOpen(false);
    } catch (cause) {
      setError(
        cause instanceof Error ? cause.message : t("workflowRun.deployFailed"),
      );
    }
  }

  function resetLocalState(): void {
    setError(null);
    setProjectId("");
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

        <div className="mt-2 space-y-2">
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
                  {mountedProjects.length > 0 && (
                    <CommandGroup heading={t("workflowRun.deployGroupMounted")}>
                      {mountedProjects.map((project) => (
                        <CommandItem
                          key={project.id}
                          value={project.name}
                          data-checked={project.id === projectId}
                          className={MENU_ITEM_CLASS}
                          onSelect={() => {
                            setProjectId(project.id);
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
                        mountedProjects.length > 0
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
              {alreadyMounted
                ? t("workflowRun.deployHintRemount")
                : t("workflowRun.deployHintFirst")}
            </p>
          )}
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
              : alreadyMounted
                ? t("workflowRun.deployConfirmAgain")
                : t("workflowRun.deployConfirmFirst")}
          </Button>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
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
