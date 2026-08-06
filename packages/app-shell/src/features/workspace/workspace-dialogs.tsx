import { useState } from "react";
import { useTranslation } from "react-i18next";
import { RemoteContractError, type ProjectBranch, type TaskStatus } from "@ora/contracts";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@ora/ui";
import { IconTrash } from "@tabler/icons-react";
import { EntityDialog, type EntityField } from "./entity-dialog";
import {
  useCreateProject,
  useUpdateProject,
  useDeleteProject,
  useCreateTask,
  useUpdateTask,
  useDeleteTask,
  useCreateSession,
  useDeleteSession,
} from "../../state/hooks/use-workspace-mutations";
import { useDeleteWorkflowRun, useRenameWorkflowRun } from "../../state/hooks/use-workflow-runs";
import { useUiStore, type DialogState, type DeleteTarget } from "../../state/stores/ui-store";
import { useSettingsStore } from "../../state/stores/settings-store";
import { localizeContractError } from "../../i18n/contract-error";
import { useProjectBranches } from "../../state/hooks/use-project-branches";
import { projectNameFromPath } from "./workspace-dialogs-utils";

/**
 * Hosts every workspace create/edit/delete dialog.
 *
 * These are driven entirely by `useUiStore`, so any surface can open one by
 * setting `dialog`/`deleteTarget`. Mounting them at the app shell rather than
 * inside the sidebar is what makes that true: the sidebar unmounts when it is
 * collapsed, which would otherwise take the dialogs down with it and silently
 * break callers such as the composer's project picker.
 */
export function WorkspaceDialogs() {
  const dialog = useUiStore((s) => s.dialog);
  const setDialog = useUiStore((s) => s.setDialog);
  const deleteTarget = useUiStore((s) => s.deleteTarget);
  const setDeleteTarget = useUiStore((s) => s.setDeleteTarget);

  return (
    <>
      {dialog && <WorkspaceEntityDialog dialog={dialog} onOpenChange={(open) => !open && setDialog(null)} />}
      <DeleteEntityDialog key={deleteTarget?.id ?? "none"} target={deleteTarget} onOpenChange={(open) => !open && setDeleteTarget(null)} />
    </>
  );
}

/** Confirms destructive tree mutations and prevents duplicate requests while cascading deletes run. */
function DeleteEntityDialog({ target, onOpenChange }: { target: DeleteTarget | null; onOpenChange: (open: boolean) => void }) {
  const { t } = useTranslation();
  const [deleting, setDeleting] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const deleteProject = useDeleteProject();
  const deleteTask = useDeleteTask();
  const deleteSession = useDeleteSession();
  const deleteWorkflowRun = useDeleteWorkflowRun();

  const confirmDelete = async () => {
    if (!target || deleting) return;
    setDeleting(true);
    setDeleteError(null);
    try {
      if (target.kind === "project") {
        // Project deletion has the same running-session guard as task deletion.
        // Stop and remove every descendant session before cascading the project.
        for (const sessionId of target.sessionIds) {
          await deleteSession.mutateAsync({ sessionId });
        }
        await deleteProject.mutateAsync({ projectId: target.id });
      }
      if (target.kind === "task") {
        // The backend protects every task with a running provider session.
        // Stop and remove each child session before deleting either task mode.
        for (const sessionId of target.sessionIds) {
          await deleteSession.mutateAsync({ sessionId });
        }
        await deleteTask.mutateAsync({ taskId: target.id });
      }
      if (target.kind === "session") await deleteSession.mutateAsync({ sessionId: target.id });
      if (target.kind === "workflowRun") {
        await deleteWorkflowRun.mutateAsync({
          runId: target.id,
          projectId: target.projectId,
        });
      }
      onOpenChange(false);
    } catch (error) {
      setDeleteError(error instanceof RemoteContractError && error.code === "resource_in_use"
        ? target.kind === "task" && target.workspaceMode === "project_root"
          ? t("delete.runningSession")
          : t("delete.failed")
        : localizeContractError(error, t));
    } finally {
      setDeleting(false);
    }
  };

  return (
    <AlertDialog open={target !== null} onOpenChange={(open) => !deleting && onOpenChange(open)}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("delete.title", { name: target?.name ?? "" })}</AlertDialogTitle>
          <AlertDialogDescription>{target
            ? target.kind === "task" && target.workspaceMode === "project_root"
              ? t("delete.directTaskDescription")
              : t(`delete.${target.kind}Description`)
            : ""}</AlertDialogDescription>
          {deleteError && <p role="alert" data-selectable className="text-sm text-destructive">{deleteError}</p>}
        </AlertDialogHeader>
        <AlertDialogFooter>
          <AlertDialogCancel disabled={deleting}>{t("common.cancel")}</AlertDialogCancel>
          <AlertDialogAction variant="destructive" disabled={deleting} onClick={() => void confirmDelete()}>
            <IconTrash />{deleting ? t("delete.deleting") : t("common.delete")}
          </AlertDialogAction>
        </AlertDialogFooter>
      </AlertDialogContent>
    </AlertDialog>
  );
}

/** Adapts the generic entity form to the selected workspace entity and mutation. */
function WorkspaceEntityDialog({ dialog, onOpenChange }: { dialog: DialogState; onOpenChange: (open: boolean) => void }) {
  const { t } = useTranslation();
  const createProject = useCreateProject();
  const updateProject = useUpdateProject();
  const createTask = useCreateTask();
  const updateTask = useUpdateTask();
  const createSession = useCreateSession();
  const renameWorkflowRun = useRenameWorkflowRun();
  const settingsAgentCli = useSettingsStore((state) => state.settings.agentCli);
  const branchProjectId = dialog.kind === "task" && !dialog.entity ? dialog.projectId : null;
  const { data: projectBranches = [] } = useProjectBranches(branchProjectId);
  let title: string;
  let description: string | undefined;
  let fields: EntityField[];
  let submitLabel: string;
  let submit: (values: Record<string, string>) => Promise<void>;

  if (dialog.kind === "project") {
    title = dialog.entity ? t("dialog.editProject") : t("dialog.addProject");
    description = undefined;
    submitLabel = dialog.entity ? t("dialog.saveProject") : t("dialog.addProject");
    fields = dialog.entity
      ? [{ kind: "text", name: "name", label: t("dialog.projectName"), value: dialog.entity.name, placeholder: t("dialog.projectNamePlaceholder") }]
      : [{ kind: "path", name: "rootPath", label: t("dialog.projectFolder"), value: "", selectionKind: "directory", placeholder: "C:\\workspace\\project" }];
    submit = async (values) => {
      if (dialog.entity) {
        await updateProject.mutateAsync({ project: dialog.entity, name: values.name! });
      } else {
        await createProject.mutateAsync({
          name: projectNameFromPath(values.rootPath!),
          rootPath: values.rootPath!,
        });
      }
    };
  } else if (dialog.kind === "task") {
    title = dialog.entity ? t("dialog.editTask") : t("dialog.createWorktree");
    description = dialog.entity ? undefined : t("dialog.worktreeDescription");
    submitLabel = dialog.entity ? t("dialog.saveTask") : t("dialog.createTask");
    fields = [
      { kind: "text", name: "title", label: t("dialog.taskTitle"), value: dialog.entity?.title ?? "" },
      ...(!dialog.entity ? [{
        kind: "select" as const,
        name: "baseBranch",
        label: t("dialog.baseBranch"),
        value: preferredBaseBranch(projectBranches),
        options: projectBranches.map((branch) => ({ label: branch.displayName, value: branch.refName })),
      }] : []),
      // Status is only meaningful once a task exists; a new task always starts at "todo".
      ...(dialog.entity ? [{ kind: "select" as const, name: "status", label: t("dialog.status"), value: dialog.entity.status, options: [
        { label: t("common.todo"), value: "todo" }, { label: t("common.doing"), value: "doing" }, { label: t("common.done"), value: "done" },
      ] }] : []),
    ];
    submit = async (values) => {
      if (dialog.entity) {
        await updateTask.mutateAsync({ task: dialog.entity, title: values.title!, status: values.status as TaskStatus });
      } else {
        await createTask.mutateAsync({
          projectId: dialog.projectId,
          title: values.title!,
          status: "todo",
          workspaceMode: "worktree",
          baseBranch: values.baseBranch!,
        });
      }
    };
  } else if (dialog.kind === "workflowRun") {
    title = t("dialog.editWorkflowRun");
    description = undefined;
    submitLabel = t("dialog.saveWorkflowRun");
    fields = [{
      kind: "text",
      name: "name",
      label: t("dialog.workflowRunName"),
      value: dialog.entity.name,
      placeholder: t("dialog.workflowRunNamePlaceholder"),
    }];
    submit = async (values) => {
      await renameWorkflowRun.mutateAsync({
        runId: dialog.entity.id,
        name: values.name!,
      });
    };
  } else {
    title = dialog.entity ? t("dialog.editSession") : t("dialog.startSession");
    description = t("dialog.sessionDescription");
    submitLabel = dialog.entity ? t("dialog.saveSession") : t("dialog.startSession");
    fields = [];
    submit = async () => {
      if (!dialog.entity) {
        await createSession.mutateAsync({ taskId: dialog.taskId, agentCli: settingsAgentCli });
      }
    };
  }

  const dialogKey = dialog.kind === "workflowRun"
    ? `${dialog.kind}-${dialog.entity.id}`
    : `${dialog.kind}-${dialog.entity?.id ?? "new"}`;

  return <EntityDialog key={dialogKey} open title={title} description={description} submitLabel={submitLabel} fields={fields} onOpenChange={onOpenChange} onSubmit={submit} />;
}

/** Prefers a fetched conventional primary branch while preserving repositories with custom defaults. */
function preferredBaseBranch(branches: ProjectBranch[]): string {
  return branches.find((branch) => branch.name === "main")?.refName
    ?? branches.find((branch) => branch.name === "master")?.refName
    ?? branches[0]?.refName
    ?? "";
}
