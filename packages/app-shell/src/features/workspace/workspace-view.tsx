import { Button, Badge } from "@ora/ui";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import {
  IconBrandGit,
  IconFolder,
  IconGitBranch,
  IconLayoutSidebarLeftExpand,
  IconPlayerPlay,
  IconSquareRoundedPlus,
} from "@tabler/icons-react";
import { useProjects } from "../../state/hooks/use-projects";
import { useTasks } from "../../state/hooks/use-tasks";
import { useSessions } from "../../state/hooks/use-sessions";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useChatStore } from "../../chat-store-context";
import { ChatView } from "../chat/chat-view";

interface WorkspaceViewProps {
  userName: string;
}

/** Shows useful project/task context until a session is selected, then opens its agent chat. */
export function WorkspaceView({ userName }: WorkspaceViewProps) {
  const { t } = useTranslation();

  const { data: projects = [] } = useProjects();
  const { data: tasks = [] } = useTasks();
  const { data: sessions = [] } = useSessions();
  const selection = useWorkspaceSelectionStore((s) => s.selection);
  const sidebarCollapsed = useUiStore((s) => s.sidebarCollapsed);
  const setSidebarCollapsed = useUiStore((s) => s.setSidebarCollapsed);

  const chatStore = useChatStore();

  const project = projects.find((item) => item.id === selection.projectId);
  const task = tasks.find((item) => item.id === selection.taskId);
  const session = sessions.find((item) => item.id === selection.sessionId);
  const agentSessionUnavailable = session?.agentSessionId === null;
  const conversation = useStore(
    chatStore,
    (state) =>
      (selection.sessionId === null
        ? undefined
        : state.conversations[selection.sessionId]),
  );

  const clearSelection = useWorkspaceSelectionStore((s) => s.clearSelection);

  // The chat pane also backs the "no selection" landing state, where there is no
  // Ora session to talk to yet and the composer stays disabled.
  const chatIsOpen = (session && task && project) || selection.projectId === null;

  if (chatIsOpen) {
    const title = task?.title ?? t("chat.newThread");
    const sendDisabled = !session || agentSessionUnavailable;
    const chatError = conversation?.error
      ?? (agentSessionUnavailable ? t("chat.agentSessionUnavailable") : null)
      ?? (session ? null : t("chat.noSessionSelected"));
    return (
      <main id="main-content" className="relative flex min-w-0 flex-1 flex-col bg-background">
        <div className="flex h-13 shrink-0 items-center gap-2 px-3 sm:px-4">
          {sidebarCollapsed && <Button variant="ghost" size="icon-sm" onClick={() => setSidebarCollapsed(false)} aria-label={t("sidebar.expand")}><IconLayoutSidebarLeftExpand /></Button>}
          <div className="min-w-0">
            <p className="truncate text-[13px] font-medium tracking-[-0.01em]">{title}</p>
            {project && session && (
              <p className="truncate text-[10px] text-muted-foreground">{project.name} / {session.agentId}</p>
            )}
          </div>
          <div className="flex-1" />
          {session && <Badge variant="outline" className="gap-1 rounded-md text-[10px]"><span className={`size-1.5 rounded-full ${session.status === "running" ? "bg-emerald-500" : "bg-zinc-400"}`} />{t(`common.${session.status}`)}</Badge>}
          <Button variant="ghost" size="icon-sm" onClick={clearSelection} aria-label={t("chat.newThread")}><IconSquareRoundedPlus /></Button>
        </div>
        <div className="min-h-0 flex-1 [&>main]:h-full">
          <ChatView
            messages={conversation?.messages ?? []}
            userName={userName}
            isResponding={conversation?.isResponding ?? false}
            error={chatError}
            disabled={sendDisabled}
            onSend={(text) => {
              if (!session || session.agentSessionId === null) return;
              void chatStore.getState().sendMessage({
                oraSessionId: session.id,
                agentSessionId: session.agentSessionId,
                text,
              }).catch(() => undefined);
            }}
          />
        </div>
      </main>
    );
  }

  return (
    <main id="main-content" className="flex min-w-0 flex-1 flex-col bg-background">
      <header className="flex h-12 items-center border-b border-border px-3">
        {sidebarCollapsed && <Button variant="ghost" size="icon-sm" onClick={() => setSidebarCollapsed(false)} aria-label={t("sidebar.expand")}><IconLayoutSidebarLeftExpand /></Button>}
        <span className="ml-1 text-xs font-medium text-muted-foreground">{t("workspace.overview")}</span>
      </header>
      <div className="flex flex-1 items-center justify-center p-6">
        <section className="w-full max-w-xl">
          <div className="mb-6 flex size-11 items-center justify-center rounded-lg border border-border bg-muted">
            {task ? <IconGitBranch className="size-5 text-sky-600" /> : <IconFolder className="size-5 text-amber-600" />}
          </div>
          <h1 className="text-xl font-semibold">{task?.title ?? project?.name ?? t("workspace.defaultTitle")}</h1>
          <p className="mt-2 max-w-md text-sm leading-6 text-muted-foreground">
            {task
              ? t("workspace.taskHint")
              : project
                ? t("workspace.projectHint")
                : t("workspace.emptyHint")}
          </p>
          {(project || task) && (
            <div className="mt-6 grid gap-px overflow-hidden rounded-md border border-border bg-border sm:grid-cols-2">
              <div className="bg-background p-4">
                <div className="flex items-center gap-2 text-xs text-muted-foreground"><IconBrandGit className="size-4" />{t("workspace.repository")}</div>
                <p className="mt-2 truncate text-sm font-medium">{project?.rootPath}</p>
              </div>
              <div className="bg-background p-4">
                <div className="flex items-center gap-2 text-xs text-muted-foreground"><IconPlayerPlay className="size-4" />{t("workspace.agentSessions")}</div>
                <p className="mt-2 text-sm font-medium">{task
                  ? t("workspace.sessionCount", { count: sessions.filter((item) => item.taskId === task.id).length })
                  : t("workspace.worktreeCount", { count: tasks.filter((item) => item.projectId === project?.id).length })}</p>
              </div>
            </div>
          )}
        </section>
      </div>
    </main>
  );
}
