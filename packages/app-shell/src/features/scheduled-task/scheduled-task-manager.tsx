import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ScheduledTask } from "@ora/contracts";
import { Button, cn, Spinner, Switch } from "@ora/ui";
import {
  IconCalendarClock,
  IconEdit,
  IconHistory,
  IconLayoutSidebarLeftExpand,
  IconPlus,
  IconShieldX,
  IconTrash,
} from "@tabler/icons-react";
import { localizeContractError } from "../../i18n/contract-error";
import { useAvailableAgents } from "../../state/hooks/use-available-agents";
import { useProjects } from "../../state/hooks/use-projects";
import { useProjectsWithoutGit } from "../../state/hooks/use-workspace-git";
import {
  useDeleteScheduledTask,
  useRevokeScheduledTaskApproval,
  useScheduledTasks,
  useUpdateScheduledTask,
} from "../../state/hooks/use-scheduled-tasks";
import { useUiStore } from "../../state/stores/ui-store";
import { ScheduledTaskConfirmDialog } from "./scheduled-task-confirm-dialog";
import { ScheduledTaskForm } from "./scheduled-task-form";
import { ScheduledTaskHistoryDialog } from "./scheduled-task-history-dialog";
import { scheduleLabel } from "./scheduled-task-format";

/** Names one pending destructive action awaiting explicit confirmation. */
interface PendingConfirm {
  kind: "delete" | "revoke";
  task: ScheduledTask;
}

/** First-release Scheduled Task definition list and editor. */
export function ScheduledTaskManager() {
  const { t } = useTranslation();
  const tasksQuery = useScheduledTasks();
  const projectsQuery = useProjects();
  const agents = useAvailableAgents();
  const updateTask = useUpdateScheduledTask();
  const deleteTask = useDeleteScheduledTask();
  const revokeApproval = useRevokeScheduledTaskApproval();
  const projectsWithoutGit = useProjectsWithoutGit();
  const sidebarCollapsed = useUiStore((state) => state.sidebarCollapsed);
  const setSidebarCollapsed = useUiStore((state) => state.setSidebarCollapsed);
  const historyRequest = useUiStore(
    (state) => state.scheduledTaskHistoryRequest,
  );
  const clearScheduledTaskHistoryRequest = useUiStore(
    (state) => state.clearScheduledTaskHistoryRequest,
  );
  const [editing, setEditing] = useState<ScheduledTask | null | undefined>();
  const [historyTask, setHistoryTask] = useState<ScheduledTask | null>(null);
  const [pendingConfirm, setPendingConfirm] = useState<PendingConfirm | null>(
    null,
  );
  const projects = useMemo(
    () => projectsQuery.data ?? [],
    [projectsQuery.data],
  );
  const tasks = tasksQuery.data ?? [];
  const projectNames = useMemo(
    () => new Map(projects.map((project) => [project.id, project.name])),
    [projects],
  );

  // A one-shot history deep link (for example from a notification) resolves against the
  // definition list at render time; closing the dialog or leaving the pane clears it, so a
  // stale request never re-opens a dialog on the next visit.
  useEffect(
    () => clearScheduledTaskHistoryRequest,
    [clearScheduledTaskHistoryRequest],
  );
  const requestedHistoryTask =
    historyRequest === null
      ? null
      : (tasks.find((task) => task.id === historyRequest.taskId) ?? null);
  const displayedHistoryTask = historyTask ?? requestedHistoryTask;

  const agentNames = useMemo(
    () => new Map(agents.map((agent) => [agent.agentRef, agent.label])),
    [agents],
  );
  const error =
    tasksQuery.error ??
    projectsQuery.error ??
    updateTask.error ??
    deleteTask.error ??
    revokeApproval.error;

  const toggleEnabled = (task: ScheduledTask, enabled: boolean) => {
    // Turning a task on needs consent and a Git project; when either is missing, the form is
    // the one surface that explains what is missing. Turning it off always stays available.
    if (
      enabled &&
      (task.approval === null || projectsWithoutGit.has(task.projectId))
    ) {
      setEditing(task);
      return;
    }
    updateTask.mutate({
      taskId: task.id,
      name: task.name,
      enabled,
      schedule: task.schedule,
      projectId: task.projectId,
      agentRef: task.agentRef,
      model: task.model,
      prompt: task.prompt,
      expectedUpdatedAt: task.updatedAt,
      // The quick toggle must never grant, refresh, or revoke unattended consent;
      // the form is the only surface that owns that decision.
      approvalChange: { kind: "keep" },
    });
  };

  return (
    <section className="flex size-full min-w-0 flex-col bg-background">
      <header className="flex items-center gap-4 border-b px-6 py-5">
        {sidebarCollapsed && (
          <Button
            variant="ghost"
            size="icon"
            onClick={() => setSidebarCollapsed(false)}
            aria-label={t("sidebar.expand")}
          >
            <IconLayoutSidebarLeftExpand />
          </Button>
        )}
        <div className="flex size-10 items-center justify-center rounded-lg bg-primary/10 text-primary">
          <IconCalendarClock className="size-5" />
        </div>
        <div className="min-w-0 flex-1">
          <h1 className="text-lg font-semibold">{t("scheduledTask.title")}</h1>
          <p className="text-sm text-muted-foreground">
            {t("scheduledTask.description")}
          </p>
        </div>
        <Button
          onClick={() => setEditing(null)}
          disabled={projects.length === 0 || agents.length === 0}
        >
          <IconPlus />
          {t("scheduledTask.new")}
        </Button>
      </header>
      <div className="min-h-0 flex-1 overflow-y-auto p-6">
        {projects.length === 0 && (
          <p className="mb-4 rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
            {t("scheduledTask.noProjects")}
          </p>
        )}
        {agents.length === 0 && (
          <p className="mb-4 rounded-lg border border-dashed p-4 text-sm text-muted-foreground">
            {t("scheduledTask.noAgents")}
          </p>
        )}
        {tasksQuery.isPending && (
          <div className="flex items-center gap-2 text-sm text-muted-foreground">
            <Spinner className="size-4" />
            {t("scheduledTask.loading")}
          </div>
        )}
        {error && (
          <p data-selectable className="text-sm text-destructive">
            {localizeContractError(error, t)}
          </p>
        )}
        {!tasksQuery.isPending && !error && tasks.length === 0 && (
          <div className="rounded-xl border border-dashed p-10 text-center text-sm text-muted-foreground">
            {t("scheduledTask.empty")}
          </div>
        )}
        <div className="grid gap-3 xl:grid-cols-2">
          {tasks.map((task) => (
            <article
              key={task.id}
              className="rounded-xl border bg-card p-4 shadow-sm"
            >
              <div className="flex items-start gap-3">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-2">
                    <h2 className="truncate font-medium">{task.name}</h2>
                    <span
                      className={cn(
                        "rounded-full px-2 py-0.5 text-[11px] font-medium",
                        task.enabled
                          ? "bg-emerald-500/10 text-emerald-600"
                          : "bg-muted text-muted-foreground",
                      )}
                    >
                      {task.enabled
                        ? t("scheduledTask.statusEnabled")
                        : t("scheduledTask.statusDisabled")}
                    </span>
                  </div>
                  <p className="mt-1 line-clamp-2 text-sm text-muted-foreground">
                    {task.prompt}
                  </p>
                </div>
                <Switch
                  checked={task.enabled}
                  disabled={updateTask.isPending}
                  onCheckedChange={(enabled) => toggleEnabled(task, enabled)}
                  aria-label={
                    task.enabled
                      ? t("scheduledTask.statusEnabled")
                      : t("scheduledTask.statusDisabled")
                  }
                />
              </div>
              <dl className="mt-4 grid gap-2 text-xs text-muted-foreground sm:grid-cols-2">
                <div>
                  <dt className="sr-only">{t("scheduledTask.schedule")}</dt>
                  <dd>{scheduleLabel(task.schedule, t)}</dd>
                </div>
                <div>
                  <dt className="sr-only">{t("scheduledTask.project")}</dt>
                  <dd className="truncate">
                    {projectNames.get(task.projectId) ?? task.projectId}
                  </dd>
                </div>
                <div>
                  <dt className="sr-only">{t("scheduledTask.agent")}</dt>
                  <dd className="truncate">
                    {agentNames.get(task.agentRef) ?? task.agentRef}
                  </dd>
                </div>
                <div>
                  <dt className="sr-only">{t("scheduledTask.model")}</dt>
                  <dd className="truncate">{task.model ?? "—"}</dd>
                </div>
              </dl>
              <div className="mt-4 flex flex-wrap justify-end gap-2 border-t pt-3">
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={() => setHistoryTask(task)}
                >
                  <IconHistory />
                  {t("scheduledTask.history")}
                </Button>
                {task.approval !== null && (
                  <Button
                    type="button"
                    size="sm"
                    variant="ghost"
                    disabled={revokeApproval.isPending}
                    onClick={() => setPendingConfirm({ kind: "revoke", task })}
                  >
                    <IconShieldX />
                    {t("scheduledTask.revoke")}
                  </Button>
                )}
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  onClick={() => setEditing(task)}
                >
                  <IconEdit />
                  {t("scheduledTask.edit")}
                </Button>
                <Button
                  type="button"
                  size="sm"
                  variant="ghost"
                  disabled={deleteTask.isPending}
                  onClick={() => setPendingConfirm({ kind: "delete", task })}
                >
                  <IconTrash />
                  {t("scheduledTask.delete")}
                </Button>
              </div>
            </article>
          ))}
        </div>
      </div>
      {editing !== undefined && (
        <ScheduledTaskForm
          key={editing?.id ?? "new"}
          open
          task={editing}
          projects={projects}
          agents={agents}
          onOpenChange={(open) => !open && setEditing(undefined)}
        />
      )}
      {displayedHistoryTask !== null && (
        <ScheduledTaskHistoryDialog
          task={displayedHistoryTask}
          onOpenChange={(open) => {
            if (!open) {
              setHistoryTask(null);
              clearScheduledTaskHistoryRequest();
            }
          }}
        />
      )}
      {pendingConfirm !== null && (
        <ScheduledTaskConfirmDialog
          title={
            pendingConfirm.kind === "delete"
              ? t("scheduledTask.deleteTitle")
              : t("scheduledTask.revoke")
          }
          description={t(
            pendingConfirm.kind === "delete"
              ? "scheduledTask.deleteConfirm"
              : "scheduledTask.revokeConfirm",
            { name: pendingConfirm.task.name },
          )}
          confirmLabel={
            pendingConfirm.kind === "delete"
              ? t("common.delete")
              : t("scheduledTask.revoke")
          }
          busy={deleteTask.isPending || revokeApproval.isPending}
          onConfirm={() => {
            if (pendingConfirm.kind === "delete") {
              deleteTask.mutate(pendingConfirm.task);
            } else {
              revokeApproval.mutate(pendingConfirm.task);
            }
            setPendingConfirm(null);
          }}
          onOpenChange={(open) => !open && setPendingConfirm(null)}
        />
      )}
    </section>
  );
}
