import { memo } from "react";
import { useTranslation } from "react-i18next";
import {
  IconFolder,
  IconFolderOpen,
  IconGitBranch,
  IconTrash,
} from "@tabler/icons-react";
import type { Project, Session, Task } from "@ora/contracts";
import type { DraftPlacement } from "../../state/stores/draft-sessions-store";
import { useUiStore } from "../../state/stores/ui-store";
import { useWorkspaceSelectionStore } from "../../state/stores/workspace-selection-store";
import { startSessionDraft } from "../../state/session-drafts";
import {
  useUpdateProject,
  useUpdateTask,
} from "../../state/hooks/use-workspace-mutations";
import { DraftSessionTreeRow } from "./draft-session-tree-row";
import { SessionTreeRow } from "./session-tree-row";
import { SidebarCreateMenu } from "./sidebar-create-menu";
import {
  NewSessionButton,
  ProjectWorkflowRunRows,
  TreeBranch,
  TreeRow,
} from "./workspace-tree-row";

const EMPTY_SESSIONS: Session[] = [];
const EMPTY_DRAFTS: DraftPlacement[] = [];

interface ProjectTreeNodeProps {
  project: Project;
  tasks: readonly Task[];
  sessionsByWorkspaceId: ReadonlyMap<string, readonly Session[]>;
  directSessions: readonly Session[];
  directDrafts: readonly DraftPlacement[];
  worktreeDraftsByTaskId: ReadonlyMap<string, readonly DraftPlacement[]>;
  /** Search forces branches open without mutating persisted expand sets. */
  forceExpanded: boolean;
}

/**
 * True when this project's visible tree props are referentially unchanged.
 *
 * The parent rebuilds the sessions/tasks Maps on any list write; only the
 * buckets for *this* project's tasks matter for memoization.
 */
function projectTreeNodePropsEqual(
  prev: ProjectTreeNodeProps,
  next: ProjectTreeNodeProps,
): boolean {
  if (prev.project !== next.project) return false;
  if (prev.tasks !== next.tasks) return false;
  if (prev.forceExpanded !== next.forceExpanded) return false;
  if (prev.directDrafts !== next.directDrafts) return false;
  if (prev.directSessions !== next.directSessions) return false;
  for (const task of next.tasks) {
    if (
      prev.sessionsByWorkspaceId.get(task.workspaceId) !==
      next.sessionsByWorkspaceId.get(task.workspaceId)
    ) {
      return false;
    }
    if (
      prev.worktreeDraftsByTaskId.get(task.id) !==
      next.worktreeDraftsByTaskId.get(task.id)
    ) {
      return false;
    }
  }
  return true;
}

/**
 * One project row and its descendants. Expand state is subscribed per id so
 * toggling another project does not reconcile this subtree.
 */
export const ProjectTreeNode = memo(function ProjectTreeNode({
  project,
  tasks,
  sessionsByWorkspaceId,
  directSessions,
  directDrafts,
  worktreeDraftsByTaskId,
  forceExpanded,
}: ProjectTreeNodeProps) {
  const { t } = useTranslation();
  const updateProject = useUpdateProject();
  const projectOpen =
    useUiStore((s) => s.expandedProjects.has(project.id)) || forceExpanded;

  const projectSelected = useWorkspaceSelectionStore(
    (s) =>
      s.selection.projectId === project.id &&
      s.selection.taskId === null &&
      s.selection.sessionId === null &&
      s.selection.draftId === null &&
      s.selection.workflowRunId === null,
  );
  const projectContainsSelection = useWorkspaceSelectionStore((s) => {
    const { selection } = s;
    return (
      selection.projectId === project.id &&
      (selection.taskId !== null ||
        selection.sessionId !== null ||
        selection.draftId !== null ||
        selection.workflowRunId !== null)
    );
  });
  const projectCreateFocused = useWorkspaceSelectionStore((s) => {
    const { createFocus, selection } = s;
    return (
      createFocus !== null &&
      createFocus.projectId === project.id &&
      createFocus.taskId === null &&
      !(
        selection.projectId === createFocus.projectId &&
        selection.taskId === createFocus.taskId
      )
    );
  });
  const activeRunId = useWorkspaceSelectionStore((s) =>
    s.selection.projectId === project.id ? s.selection.workflowRunId : null,
  );

  const projectSessionIds = [
    ...directSessions.map((session) => session.id),
    ...tasks.flatMap((task) =>
      (sessionsByWorkspaceId.get(task.workspaceId) ?? EMPTY_SESSIONS).map(
        (session) => session.id,
      ),
    ),
  ];

  return (
    <div>
      <TreeRow
        depth={0}
        active={projectSelected}
        containsSelection={!projectOpen && projectContainsSelection}
        createFocused={projectCreateFocused}
        icon={
          projectOpen ? (
            <IconFolderOpen className="size-[18px] text-muted-foreground" />
          ) : (
            <IconFolder className="size-[18px] text-muted-foreground" />
          )
        }
        label={project.name}
        expanded={projectOpen}
        onClick={() => {
          useWorkspaceSelectionStore
            .getState()
            .setCreateFocus({ projectId: project.id, taskId: null });
          useUiStore.getState().toggleProjectExpand(project.id);
        }}
        action={
          <SidebarCreateMenu
            projectId={project.id}
            onNewTask={(projectId) => {
              startSessionDraft({ projectId, taskId: null });
            }}
          />
        }
        onRename={(name) => updateProject.mutateAsync({ project, name })}
        commands={[
          {
            label: t("common.delete"),
            icon: <IconTrash />,
            variant: "destructive",
            onSelect: () =>
              useUiStore.getState().setDeleteTarget({
                kind: "project",
                id: project.id,
                name: project.name,
                sessionIds: projectSessionIds,
              }),
          },
        ]}
      />
      <TreeBranch expanded={projectOpen} retainWhenCollapsed={!forceExpanded}>
        <ProjectWorkflowRunRows
          projectId={project.id}
          listEnabled={projectOpen}
          activeRunId={activeRunId}
          onSelectRun={(runId) =>
            useWorkspaceSelectionStore
              .getState()
              .selectWorkflowRun(runId, project.id)
          }
          onDeleteRun={(run) =>
            useUiStore.getState().setDeleteTarget({
              kind: "workflowRun",
              id: run.id,
              name: run.name,
              projectId: project.id,
            })
          }
        />
        {directDrafts.map((draft) => (
          <DraftSessionTreeRow key={draft.id} draftId={draft.id} depth={1} />
        ))}
        {directSessions.map((session) => (
          <SessionTreeRow
            key={session.id}
            sessionId={session.id}
            taskId={null}
            projectId={project.id}
            depth={1}
            title={session.title ?? t("sidebar.newSession")}
            deleteAs="session"
          />
        ))}
        {tasks.map((task) => {
          const taskSessions =
            sessionsByWorkspaceId.get(task.workspaceId) ?? EMPTY_SESSIONS;
          return (
            <WorktreeTaskNode
              key={task.id}
              task={task}
              projectId={project.id}
              sessions={taskSessions}
              drafts={worktreeDraftsByTaskId.get(task.id) ?? EMPTY_DRAFTS}
              forceExpanded={forceExpanded}
            />
          );
        })}
      </TreeBranch>
    </div>
  );
}, projectTreeNodePropsEqual);

interface WorktreeTaskNodeProps {
  task: Task;
  projectId: string;
  sessions: readonly Session[];
  drafts: readonly DraftPlacement[];
  forceExpanded: boolean;
}

/**
 * Worktree task row with its own expand subscription so collapsing one task
 * does not rebuild sibling task session lists.
 */
const WorktreeTaskNode = memo(function WorktreeTaskNode({
  task,
  projectId,
  sessions,
  drafts,
  forceExpanded,
}: WorktreeTaskNodeProps) {
  const { t } = useTranslation();
  const updateTask = useUpdateTask();
  const taskOpen =
    useUiStore((s) => s.expandedTasks.has(task.id)) || forceExpanded;

  const taskSelected = useWorkspaceSelectionStore(
    (s) =>
      s.selection.taskId === task.id &&
      s.selection.sessionId === null &&
      s.selection.draftId === null,
  );
  const taskContainsSelection = useWorkspaceSelectionStore(
    (s) =>
      s.selection.taskId === task.id &&
      (s.selection.sessionId !== null || s.selection.draftId !== null),
  );
  const taskCreateFocused = useWorkspaceSelectionStore((s) => {
    const { createFocus, selection } = s;
    return (
      createFocus?.taskId === task.id &&
      !(
        selection.projectId === createFocus.projectId &&
        selection.taskId === createFocus.taskId
      )
    );
  });

  return (
    <div>
      <TreeRow
        depth={1}
        active={taskSelected}
        containsSelection={!taskOpen && taskContainsSelection}
        createFocused={taskCreateFocused}
        icon={
          <IconGitBranch
            className="size-4 text-muted-foreground"
            aria-label={t("sidebar.worktreeTask")}
          />
        }
        label={task.title}
        expanded={taskOpen}
        onClick={() => {
          useWorkspaceSelectionStore
            .getState()
            .setCreateFocus({ projectId, taskId: task.id });
          useUiStore.getState().toggleTaskExpand(task.id);
        }}
        action={
          <NewSessionButton
            onClick={() =>
              startSessionDraft({
                projectId: task.projectId,
                taskId: task.id,
              })
            }
          />
        }
        onRename={(name) => updateTask.mutateAsync({ task, title: name })}
        commands={[
          {
            label: t("common.delete"),
            icon: <IconTrash />,
            variant: "destructive",
            onSelect: () =>
              useUiStore.getState().setDeleteTarget({
                kind: "task",
                id: task.id,
                name: task.title,
                sessionIds: sessions.map((session) => session.id),
              }),
          },
        ]}
      />
      <TreeBranch expanded={taskOpen} retainWhenCollapsed={!forceExpanded}>
        {drafts.map((draft) => (
          <DraftSessionTreeRow key={draft.id} draftId={draft.id} depth={2} />
        ))}
        {sessions.map((session) => (
          <SessionTreeRow
            key={session.id}
            sessionId={session.id}
            taskId={task.id}
            projectId={projectId}
            depth={2}
            title={session.title ?? t("sidebar.newSession")}
            deleteAs="session"
          />
        ))}
      </TreeBranch>
    </div>
  );
});
