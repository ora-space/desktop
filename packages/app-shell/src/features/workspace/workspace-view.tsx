import { useEffect } from "react";
import { Button } from "@ora/ui";
import { useTranslation } from "react-i18next";
import { useStore } from "zustand";
import {
  IconBrandGit,
  IconFolder,
  IconGitBranch,
  IconLayoutSidebarLeftExpand,
  IconPlayerPlay,
} from "@tabler/icons-react";
import { useProjects } from "../../state/hooks/use-projects";
import { useTasks } from "../../state/hooks/use-tasks";
import { useSessions } from "../../state/hooks/use-sessions";
import { useCreateSession } from "../../state/hooks/use-workspace-mutations";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { useChatStore } from "../../chat-store-context";
import { DragRegion } from "../../components/drag-region";
import { WindowControls } from "../../components/window-controls";
import { ChatView } from "../chat/chat-view";
import { ComposerContextBar } from "../chat/composer-context-bar";

interface WorkspaceViewProps {
  userName: string;
}

/** Shows useful project/task context until a session is selected, then opens its agent chat. */
export function WorkspaceView({ userName }: WorkspaceViewProps) {
  const { t } = useTranslation();

  const { data: projects = [] } = useProjects();
  const { data: tasks = [] } = useTasks();
  const sessionsQuery = useSessions();
  const sessions = sessionsQuery.data ?? [];
  const selection = useWorkspaceSelectionStore((s) => s.selection);
  const sidebarCollapsed = useUiStore((s) => s.sidebarCollapsed);
  const setSidebarCollapsed = useUiStore((s) => s.setSidebarCollapsed);

  const chatStore = useChatStore();

  const project = projects.find((item) => item.id === selection.projectId);
  const task = tasks.find((item) => item.id === selection.taskId);
  const session = sessions.find((item) => item.id === selection.sessionId);
  const conversation = useStore(
    chatStore,
    (state) =>
      (selection.sessionId === null
        ? undefined
        : state.conversations[selection.sessionId]),
  );

  const createSession = useCreateSession();

  useEffect(() => {
    if (
      session !== undefined &&
      conversation?.isLoading !== true &&
      conversation?.isLoaded !== true &&
      conversation?.error == null
    ) {
      // A browser refresh replaces the in-memory chat store without stopping the backend-owned
      // process, so a selected session can still be Running while its local history is empty.
      void chatStore.getState().loadSession(session.id)
        .then(() => sessionsQuery.refetch())
        .catch(() => undefined);
    }
  }, [chatStore, conversation?.error, conversation?.isLoaded, conversation?.isLoading, session?.id, session?.status, sessionsQuery]);

  /**
   * Sends into the selected session, or starts one for the selected worktree
   * first. useCreateSession already opens the agent session against the
   * project's root path and selects the result, so the implicit path here is the
   * same one the session dialog takes.
   */
  const sendOrStartSession = async (text: string) => {
    const target = session ?? (task
      ? await createSession.mutateAsync({ taskId: task.id })
      : undefined);
    if (target === undefined) return;
    try {
      await chatStore.getState().sendMessage({
        oraSessionId: target.id,
        text,
      });
    } finally {
      // Connection failures can stop the provider process, so refresh the persisted lifecycle
      // snapshot after every finite prompt without polling all idle sessions.
      await sessionsQuery.refetch();
    }
  };

  // Anything short of a selected session is the new-task landing. The composer's
  // context bar owns the project and branch selection, so choosing either must not
  // navigate away from the composer that reads them. The overview is left as the
  // fallback for a session whose task or project has gone missing.
  const chatIsOpen = session === undefined || (task !== undefined && project !== undefined);

  if (chatIsOpen) {
    // With a session selected the agent session decides; without one, a project and
    // worktree are enough, because the first message creates the session itself.
    const canChat = session
      ? session.status === "running" || conversation?.isLoaded === true
      : task !== undefined && project !== undefined;
    const chatError = conversation?.error
      ?? createSession.error?.message
      ?? null;
    return (
      <main id="main-content" className="relative flex min-h-0 min-w-0 flex-1 flex-col bg-background">
        <div className="flex h-14 shrink-0 items-center gap-2 px-3 sm:px-4">
          {sidebarCollapsed && <Button variant="ghost" size="icon" onClick={() => setSidebarCollapsed(false)} aria-label={t("sidebar.expand")}><IconLayoutSidebarLeftExpand /></Button>}
          <DragRegion>
            {session && (
              <div className="min-w-0">
                <p className="truncate text-sm font-medium tracking-[-0.01em]">{session.agentCli}</p>
                {project && task && (
                  <p className="truncate text-[11px] text-muted-foreground">{project.name} / {task.title}</p>
                )}
              </div>
            )}
          </DragRegion>
          <WindowControls />
        </div>
        <div className="flex min-h-0 flex-1 flex-col">
          <ChatView
            turns={conversation?.turns ?? []}
            userName={userName}
            isResponding={conversation?.isResponding ?? false}
            error={chatError}
            pendingPermissions={conversation?.pendingPermissions ?? []}
            disabled={!canChat}
            disabledHint={canChat ? undefined : t("chat.pickProjectAndBranch")}
            // A live session already fixes its project and branch, so the pickers
            // only belong to the not-yet-created task.
            contextBar={session ? undefined : <ComposerContextBar />}
            // Failures land in chatError; the rejection itself is expected.
            onSend={(text) => void sendOrStartSession(text).catch(() => undefined)}
            onStop={() => chatStore.getState().stopGeneration(session?.id ?? "")}
            onRespondToPermission={(permissionRequestId, optionId) => {
              if (session) {
                void chatStore.getState()
                  .respondToPermission(session.id, permissionRequestId, optionId)
                  .catch(() => undefined);
              }
            }}
          />
        </div>
      </main>
    );
  }

  return (
    <main id="main-content" className="flex min-h-0 min-w-0 flex-1 flex-col bg-background">
      <header className="flex h-14 items-center border-b border-border px-3">
        {sidebarCollapsed && <Button variant="ghost" size="icon" onClick={() => setSidebarCollapsed(false)} aria-label={t("sidebar.expand")}><IconLayoutSidebarLeftExpand /></Button>}
        <DragRegion>
          <span className="text-[13px] font-medium text-muted-foreground">{t("workspace.overview")}</span>
        </DragRegion>
        <WindowControls />
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
