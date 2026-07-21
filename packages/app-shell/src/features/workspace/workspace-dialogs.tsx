import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { SessionStatus, TaskStatus } from "@ora/contracts";
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
  useUpdateSession,
  useDeleteSession,
} from "../../state/hooks/use-workspace-mutations";
import { useUiStore, type DialogState, type DeleteTarget } from "../../state/stores/ui-store";

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
      <DeleteEntityDialog target={deleteTarget} onOpenChange={(open) => !open && setDeleteTarget(null)} />
    </>
  );
}

/** Confirms destructive tree mutations and prevents duplicate requests while cascading deletes run. */
function DeleteEntityDialog({ target, onOpenChange }: { target: DeleteTarget | null; onOpenChange: (open: boolean) => void }) {
  const { t } = useTranslation();
  const [deleting, setDeleting] = useState(false);
  const deleteProject = useDeleteProject();
  const deleteTask = useDeleteTask();
  const deleteSession = useDeleteSession();

  const confirmDelete = async () => {
    if (!target || deleting) return;
    setDeleting(true);
    try {
      if (target.kind === "project") await deleteProject.mutateAsync({ projectId: target.id });
      if (target.kind === "task") await deleteTask.mutateAsync({ taskId: target.id });
      if (target.kind === "session") await deleteSession.mutateAsync({ sessionId: target.id });
      onOpenChange(false);
    } catch {
      // The sidebar error banner surfaces transport errors; the dialog stays open for retry.
    } finally {
      setDeleting(false);
    }
  };

  return (
    <AlertDialog open={target !== null} onOpenChange={(open) => !deleting && onOpenChange(open)}>
      <AlertDialogContent>
        <AlertDialogHeader>
          <AlertDialogTitle>{t("delete.title", { name: target?.name ?? "" })}</AlertDialogTitle>
          <AlertDialogDescription>{target ? t(`delete.${target.kind}Description`) : ""}</AlertDialogDescription>
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
  const updateSession = useUpdateSession();
  let title: string;
  let description: string;
  let fields: EntityField[];
  let submitLabel: string;
  let submit: (values: Record<string, string>) => Promise<void>;

  if (dialog.kind === "project") {
    title = dialog.entity ? t("dialog.editProject") : t("dialog.addProject");
    description = t("dialog.projectDescription");
    submitLabel = dialog.entity ? t("dialog.saveProject") : t("dialog.addProject");
    fields = [
      { kind: "text", name: "name", label: t("dialog.projectName"), value: dialog.entity?.name ?? "", placeholder: t("dialog.projectNamePlaceholder") },
      { kind: "path", name: "rootPath", label: t("dialog.repositoryPath"), value: dialog.entity?.rootPath ?? "", selectionKind: "directory", placeholder: "C:\\workspace\\project" },
    ];
    submit = async (values) => {
      if (dialog.entity) {
        await updateProject.mutateAsync({ project: dialog.entity, name: values.name!, rootPath: values.rootPath! });
      } else {
        await createProject.mutateAsync({ name: values.name!, rootPath: values.rootPath! });
      }
    };
  } else if (dialog.kind === "task") {
    title = dialog.entity ? t("dialog.editWorktree") : t("dialog.createWorktree");
    description = t("dialog.worktreeDescription");
    submitLabel = dialog.entity ? t("dialog.saveTask") : t("dialog.createTask");
    fields = [
      { kind: "text", name: "title", label: t("dialog.taskTitle"), value: dialog.entity?.title ?? "", placeholder: t("dialog.taskPlaceholder") },
      { kind: "select", name: "status", label: t("dialog.status"), value: dialog.entity?.status ?? "todo", options: [
        { label: t("common.todo"), value: "todo" }, { label: t("common.doing"), value: "doing" }, { label: t("common.done"), value: "done" },
      ] },
    ];
    submit = async (values) => {
      if (dialog.entity) {
        await updateTask.mutateAsync({ task: dialog.entity, title: values.title!, status: values.status as TaskStatus });
      } else {
        await createTask.mutateAsync({ projectId: dialog.projectId, title: values.title!, status: values.status as TaskStatus });
      }
    };
  } else {
    title = dialog.entity ? t("dialog.editSession") : t("dialog.startSession");
    description = t("dialog.sessionDescription");
    submitLabel = dialog.entity ? t("dialog.saveSession") : t("dialog.startSession");
    fields = [
      { kind: "text", name: "agentId", label: t("dialog.agent"), value: dialog.entity?.agentId ?? "codex", placeholder: "codex" },
      { kind: "select", name: "status", label: t("dialog.status"), value: dialog.entity?.status ?? "running", options: [
        { label: t("common.running"), value: "running" }, { label: t("common.stopped"), value: "stopped" },
      ] },
    ];
    submit = async (values) => {
      if (dialog.entity) {
        await updateSession.mutateAsync({ session: dialog.entity, agentId: values.agentId!, status: values.status as SessionStatus });
      } else {
        await createSession.mutateAsync({ taskId: dialog.taskId, agentId: values.agentId!, status: values.status as SessionStatus });
      }
    };
  }

  const dialogKey = `${dialog.kind}-${dialog.entity?.id ?? "new"}`;

  return <EntityDialog key={dialogKey} open title={title} description={description} submitLabel={submitLabel} fields={fields} onOpenChange={onOpenChange} onSubmit={submit} />;
}
