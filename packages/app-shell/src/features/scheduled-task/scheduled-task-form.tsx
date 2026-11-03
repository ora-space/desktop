import { memo, useCallback, useMemo, useState, type FormEvent } from "react";
import { useTranslation } from "react-i18next";
import type {
  Project,
  ScheduledTask,
  ScheduledTaskSchedule,
  ScheduledTaskWeekday,
} from "@ora/contracts";
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  Label,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
  Spinner,
  Switch,
  Textarea,
} from "@ora/ui";
import { SearchableChoice } from "./searchable-choice";
import { localizeContractError } from "../../i18n/contract-error";
import {
  useCreateScheduledTask,
  useUpdateScheduledTask,
} from "../../state/hooks/use-scheduled-tasks";
import type { AgentEntry } from "../../state/hooks/use-agent-catalog";
import {
  WEEKDAYS,
  scheduleKindItems,
  scheduleTime,
} from "./scheduled-task-format";

interface ScheduledTaskFormProps {
  open: boolean;
  task: ScheduledTask | null;
  projects: Project[];
  agents: AgentEntry[];
  onOpenChange(open: boolean): void;
}

/** Isolated so submit/validation re-renders cannot remount Base UI Select. */
const ScheduleKindSelect = memo(function ScheduleKindSelect({
  value,
  items,
  onValueChange,
}: {
  value: ScheduledTaskSchedule["kind"];
  items: Record<ScheduledTaskSchedule["kind"], string>;
  onValueChange: (kind: ScheduledTaskSchedule["kind"]) => void;
}) {
  return (
    <Select
      value={value}
      items={items}
      onValueChange={(next) => {
        if (next === "every_hours" || next === "daily" || next === "weekly") {
          onValueChange(next);
        }
      }}
    >
      <SelectTrigger id="scheduled-task-schedule" className="w-full">
        <SelectValue>{items[value]}</SelectValue>
      </SelectTrigger>
      <SelectContent>
        <SelectItem value="every_hours">{items.every_hours}</SelectItem>
        <SelectItem value="daily">{items.daily}</SelectItem>
        <SelectItem value="weekly">{items.weekly}</SelectItem>
      </SelectContent>
    </Select>
  );
});

/** Creates or edits the bounded local Scheduled Task definition shape. */
export function ScheduledTaskForm({
  open,
  task,
  projects,
  agents,
  onOpenChange,
}: ScheduledTaskFormProps) {
  const { t, i18n } = useTranslation();
  const createTask = useCreateScheduledTask();
  const updateTask = useUpdateScheduledTask();
  const initialSchedule = task?.schedule ?? {
    kind: "every_hours" as const,
    interval: 24,
  };
  const [name, setName] = useState(task?.name ?? "");
  const [prompt, setPrompt] = useState(task?.prompt ?? "");
  const [projectId, setProjectId] = useState(
    task?.projectId ?? projects[0]?.id ?? "",
  );
  const [agentRef, setAgentRef] = useState(
    task?.agentRef ?? agents[0]?.agentRef ?? "",
  );
  const [model, setModel] = useState(task?.model ?? "");
  const [scheduleKind, setScheduleKind] = useState(initialSchedule.kind);
  const [interval, setInterval] = useState(
    initialSchedule.kind === "every_hours" ? initialSchedule.interval : 24,
  );
  const [time, setTime] = useState(scheduleTime(initialSchedule));
  const [weekdays, setWeekdays] = useState<ScheduledTaskWeekday[]>(
    initialSchedule.kind === "weekly" ? initialSchedule.weekdays : ["monday"],
  );
  const [enabled, setEnabled] = useState(task?.enabled ?? false);
  // A new definition never ships with pre-checked unattended consent: the user must opt in.
  const [approved, setApproved] = useState(
    task !== null && task.approval !== null,
  );
  const [validationError, setValidationError] = useState(false);
  const [submissionError, setSubmissionError] = useState<string | null>(null);
  const pending = createTask.isPending || updateTask.isPending;
  const agentEntries = useMemo(
    () =>
      task !== null && !agents.some((agent) => agent.agentRef === task.agentRef)
        ? [
            ...agents,
            { agentRef: task.agentRef, label: task.agentRef, logo: null },
          ]
        : agents,
    [agents, task],
  );
  const projectOptions = useMemo(
    () =>
      projects.map((project) => ({
        value: project.id,
        label: project.name.trim() === "" ? project.id : project.name,
        keywords: project.id,
      })),
    [projects],
  );
  const agentOptions = useMemo(
    () =>
      agentEntries.map((agent) => ({
        value: agent.agentRef,
        label: agent.label.trim() === "" ? agent.agentRef : agent.label,
        keywords: agent.agentRef,
      })),
    [agentEntries],
  );
  const kindItems = useMemo(() => scheduleKindItems(t), [i18n.language, t]);
  const onScheduleKindChange = useCallback(
    (kind: ScheduledTaskSchedule["kind"]) => setScheduleKind(kind),
    [],
  );

  const toggleWeekday = (weekday: ScheduledTaskWeekday) => {
    setWeekdays((current) =>
      current.includes(weekday)
        ? current.filter((candidate) => candidate !== weekday)
        : [...current, weekday],
    );
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const [hourText, minuteText] = time.split(":");
    const hour = Number(hourText);
    const minute = Number(minuteText);
    const invalid =
      name.trim() === "" ||
      prompt.trim() === "" ||
      projectId === "" ||
      agentRef === "" ||
      (enabled && !approved) ||
      (scheduleKind === "every_hours" && interval < 1) ||
      (scheduleKind === "weekly" && weekdays.length === 0) ||
      (scheduleKind !== "every_hours" &&
        (!Number.isInteger(hour) || !Number.isInteger(minute)));
    if (invalid) {
      setValidationError(true);
      return;
    }
    const schedule: ScheduledTaskSchedule =
      scheduleKind === "every_hours"
        ? { kind: "every_hours", interval }
        : scheduleKind === "daily"
          ? { kind: "daily", hour, minute }
          : { kind: "weekly", weekdays, hour, minute };
    setValidationError(false);
    setSubmissionError(null);
    try {
      const common = {
        name: name.trim(),
        enabled,
        schedule,
        projectId,
        agentRef,
        model: model.trim() || null,
        prompt: prompt.trim(),
      };
      if (task === null) {
        await createTask.mutateAsync({
          ...common,
          executionLocation: "local",
          approvalPolicy: approved ? ("full_auto_in_worktree" as const) : null,
        });
      } else {
        await updateTask.mutateAsync({
          ...common,
          taskId: task.id,
          expectedUpdatedAt: task.updatedAt,
          // Only an active re-check is a new consent fact. Echoing the stored
          // approval must neither refresh its consentedAt nor resurrect a
          // revoked one, and unchecking revokes it explicitly.
          approvalChange: approved
            ? task.approval !== null && projectId === task.projectId
              ? ({ kind: "keep" } as const)
              : ({ kind: "grant", policy: "full_auto_in_worktree" } as const)
            : ({ kind: "revoke" } as const),
        });
      }
      onOpenChange(false);
    } catch (error) {
      setSubmissionError(localizeContractError(error, t));
    }
  };

  return (
    <Dialog open={open} onOpenChange={(next) => !pending && onOpenChange(next)}>
      <DialogContent className="flex max-h-[86vh] flex-col gap-0 overflow-hidden p-0 sm:max-w-xl">
        <form onSubmit={submit} className="contents">
          <DialogHeader className="shrink-0 border-b px-4 py-3 pr-10">
            <DialogTitle>
              {task === null ? t("scheduledTask.new") : t("scheduledTask.edit")}
            </DialogTitle>
            <DialogDescription>
              {t("scheduledTask.description")}
            </DialogDescription>
          </DialogHeader>
          <div className="grid min-h-0 flex-1 gap-4 overflow-y-auto px-4 py-4">
            <div className="grid gap-1.5">
              <Label htmlFor="scheduled-task-name" className="font-normal">
                {t("scheduledTask.name")}
              </Label>
              <Input
                id="scheduled-task-name"
                value={name}
                onChange={(event) => setName(event.target.value)}
                autoFocus
              />
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="scheduled-task-prompt" className="font-normal">
                {t("scheduledTask.prompt")}
              </Label>
              <Textarea
                id="scheduled-task-prompt"
                value={prompt}
                rows={5}
                onChange={(event) => setPrompt(event.target.value)}
              />
            </div>
            <div className="grid gap-4">
              <div className="grid min-w-0 gap-1.5">
                <Label htmlFor="scheduled-task-project" className="font-normal">
                  {t("scheduledTask.project")}
                </Label>
                <SearchableChoice
                  id="scheduled-task-project"
                  value={projectId}
                  options={projectOptions}
                  placeholder={t("scheduledTask.searchProject")}
                  emptyLabel={t("scheduledTask.noMatchingProjects")}
                  onValueChange={(nextProjectId) => {
                    setProjectId(nextProjectId);
                    // Consent is scoped to the selected project. Switching a
                    // project therefore requires an intentional new opt-in.
                    if (task !== null && nextProjectId !== task.projectId) {
                      setApproved(false);
                    }
                  }}
                />
              </div>
              <div className="grid min-w-0 gap-1.5">
                <Label htmlFor="scheduled-task-agent" className="font-normal">
                  {t("scheduledTask.agent")}
                </Label>
                <SearchableChoice
                  id="scheduled-task-agent"
                  value={agentRef}
                  options={agentOptions}
                  placeholder={t("scheduledTask.searchAgent")}
                  emptyLabel={t("scheduledTask.noMatchingAgents")}
                  onValueChange={setAgentRef}
                />
              </div>
            </div>
            <div className="grid gap-1.5">
              <Label htmlFor="scheduled-task-model" className="font-normal">
                {t("scheduledTask.model")}
              </Label>
              <Input
                id="scheduled-task-model"
                value={model}
                onChange={(event) => setModel(event.target.value)}
              />
            </div>
            <div className="grid gap-2">
              <Label htmlFor="scheduled-task-schedule" className="font-normal">
                {t("scheduledTask.schedule")}
              </Label>
              <ScheduleKindSelect
                value={scheduleKind}
                items={kindItems}
                onValueChange={onScheduleKindChange}
              />
              {scheduleKind === "every_hours" ? (
                <div className="grid gap-1.5">
                  <Label
                    htmlFor="scheduled-task-interval"
                    className="font-normal"
                  >
                    {t("scheduledTask.interval")}
                  </Label>
                  <Input
                    id="scheduled-task-interval"
                    type="number"
                    min={1}
                    value={interval}
                    onChange={(event) =>
                      setInterval(Number(event.target.value))
                    }
                  />
                </div>
              ) : (
                <div className="grid gap-1.5">
                  <Label htmlFor="scheduled-task-time" className="font-normal">
                    {t("scheduledTask.time")}
                  </Label>
                  <Input
                    id="scheduled-task-time"
                    type="time"
                    value={time}
                    onChange={(event) => setTime(event.target.value)}
                  />
                </div>
              )}
              {scheduleKind === "weekly" && (
                <div className="grid gap-1.5">
                  <Label className="font-normal">
                    {t("scheduledTask.weekdays")}
                  </Label>
                  <div className="flex flex-wrap gap-1.5">
                    {WEEKDAYS.map((day) => {
                      const selected = weekdays.includes(day.value);
                      return (
                        <Button
                          key={day.value}
                          type="button"
                          size="sm"
                          variant={selected ? "secondary" : "outline"}
                          aria-pressed={selected}
                          onClick={() => toggleWeekday(day.value)}
                        >
                          {t(day.key)}
                        </Button>
                      );
                    })}
                  </div>
                </div>
              )}
            </div>
            <div className="grid gap-2 rounded-lg border p-3">
              <div className="flex items-center justify-between gap-3">
                <span className="text-sm font-medium">
                  {t("scheduledTask.local")}
                </span>
                <span className="text-xs text-muted-foreground">
                  {t("scheduledTask.cloud")}
                </span>
              </div>
              <div className="flex items-center justify-between gap-3">
                <Label
                  htmlFor="scheduled-task-approval"
                  className="cursor-pointer leading-snug"
                >
                  {t("scheduledTask.approval")}
                </Label>
                <Switch
                  id="scheduled-task-approval"
                  checked={approved}
                  onCheckedChange={setApproved}
                />
              </div>
              <p className="text-xs text-muted-foreground">
                {t("scheduledTask.approvalHelp")}
              </p>
              <div className="flex items-center justify-between gap-3">
                <Label
                  htmlFor="scheduled-task-enabled"
                  className="cursor-pointer"
                >
                  {t("scheduledTask.enabled")}
                </Label>
                <Switch
                  id="scheduled-task-enabled"
                  checked={enabled}
                  onCheckedChange={setEnabled}
                />
              </div>
            </div>
          </div>
          <DialogFooter className="mx-0 mb-0 shrink-0 flex-col items-stretch gap-2 sm:flex-col">
            <div className="min-h-5 px-1">
              {validationError ? (
                <p className="text-sm text-destructive">
                  {t("scheduledTask.validation")}
                </p>
              ) : submissionError !== null ? (
                <p data-selectable className="text-sm text-destructive">
                  {submissionError}
                </p>
              ) : null}
            </div>
            <Button
              type="submit"
              aria-busy={pending}
              className="relative self-end"
              disabled={
                pending ||
                projects.length === 0 ||
                (task === null && agents.length === 0)
              }
            >
              <span className={pending ? "opacity-0" : undefined}>
                {task === null
                  ? t("scheduledTask.create")
                  : t("scheduledTask.save")}
              </span>
              {pending ? (
                <Spinner
                  aria-hidden="true"
                  className="pointer-events-none absolute top-1/2 left-1/2 size-3.5 -translate-x-1/2 -translate-y-1/2"
                />
              ) : null}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
