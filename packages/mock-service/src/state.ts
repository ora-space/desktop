import type {
  Project,
  ProjectWorkContext,
  Session,
  Task,
} from "@ora/contracts";

export interface MockState {
  projects: Project[];
  tasks: Task[];
  sessions: Session[];
  projectWorkContexts: ProjectWorkContext[];
}

/** Creates a fresh in-memory dataset for one mock-service runtime. */
export function createInitialMockState(now = Date.now()): MockState {
  return {
    projects: [
      {
        id: "project-ora-desktop",
        name: "Ora Desktop",
        rootPath: "C:\\workspace\\ora-desktop",
      },
      {
        id: "project-design-system",
        name: "Design System",
        rootPath: "C:\\workspace\\design-system",
      },
    ],
    tasks: [
      {
        id: "task-agent-runtime",
        projectId: "project-ora-desktop",
        title: "Refactor the agent runtime",
        status: "doing",
      },
      {
        id: "task-web-layout",
        projectId: "project-ora-desktop",
        title: "Design the web client layout",
        status: "todo",
      },
      {
        id: "task-component-audit",
        projectId: "project-design-system",
        title: "Audit shared components",
        status: "done",
      },
    ],
    sessions: [
      {
        id: "session-agent-runtime",
        taskId: "task-agent-runtime",
        agentId: "codex",
        agentSessionId: "agent-session-runtime",
        status: "running",
      },
      {
        id: "session-component-audit",
        taskId: "task-component-audit",
        agentId: "codex",
        agentSessionId: null,
        status: "stopped",
      },
    ],
    projectWorkContexts: [
      {
        id: "project-work-context-web",
        surface: "web",
        windowId: "prototype-window",
        projectId: "project-ora-desktop",
        leaseExpiresAt: now + 120_000,
      },
    ],
  };
}

/** Owns the mutable arrays shared by every handler in one browser runtime. */
export const mockState = createInitialMockState();
