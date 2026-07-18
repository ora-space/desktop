import type {
  Agent,
  ListDirectoryResponse,
  Project,
  ProjectWorkContext,
  Session,
  Skill,
  Task,
} from "@ora/contracts";

export interface MockState {
  projects: Project[];
  tasks: Task[];
  sessions: Session[];
  skills: Skill[];
  agents: Agent[];
  projectWorkContexts: ProjectWorkContext[];
  homeDirectory: string;
  fileSystemDirectories: Record<string, ListDirectoryResponse>;
}

/** Creates a fresh in-memory dataset for one mock-service runtime. */
export function createInitialMockState(now = Date.now()): MockState {
  return {
    projects: [
      {
        id: "project-ora-desktop",
        name: "Ora Desktop",
        rootPath: "/home/ora/projects/ora-desktop",
      },
      {
        id: "project-design-system",
        name: "Design System",
        rootPath: "/home/ora/projects/design-system",
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
    skills: [
      {
        id: "skill-code-review",
        name: "code-review",
        description: "Reviews changes for correctness and maintainability.",
      },
    ],
    agents: [
      {
        id: "agent-codex",
        name: "Codex",
        description: "General-purpose coding agent.",
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
    homeDirectory: "/home/ora",
    fileSystemDirectories: {
      "/home/ora": {
        currentPath: "/home/ora",
        parentPath: "/home",
        breadcrumbs: [
          { name: "/", path: "/" },
          { name: "home", path: "/home" },
          { name: "ora", path: "/home/ora" },
        ],
        entries: [
          {
            name: ".config",
            path: "/home/ora/.config",
            kind: "directory",
            isSymbolicLink: false,
          },
          {
            name: "projects",
            path: "/home/ora/projects",
            kind: "directory",
            isSymbolicLink: false,
          },
          {
            name: "README.md",
            path: "/home/ora/README.md",
            kind: "file",
            isSymbolicLink: false,
          },
        ],
      },
      "/home/ora/.config": {
        currentPath: "/home/ora/.config",
        parentPath: "/home/ora",
        breadcrumbs: [
          { name: "/", path: "/" },
          { name: "home", path: "/home" },
          { name: "ora", path: "/home/ora" },
          { name: ".config", path: "/home/ora/.config" },
        ],
        entries: [],
      },
      "/home/ora/projects": {
        currentPath: "/home/ora/projects",
        parentPath: "/home/ora",
        breadcrumbs: [
          { name: "/", path: "/" },
          { name: "home", path: "/home" },
          { name: "ora", path: "/home/ora" },
          { name: "projects", path: "/home/ora/projects" },
        ],
        entries: [
          {
            name: "design-system",
            path: "/home/ora/projects/design-system",
            kind: "directory",
            isSymbolicLink: false,
          },
          {
            name: "ora-desktop",
            path: "/home/ora/projects/ora-desktop",
            kind: "directory",
            isSymbolicLink: true,
          },
        ],
      },
      "/home/ora/projects/ora-desktop": {
        currentPath: "/home/ora/projects/ora-desktop",
        parentPath: "/home/ora/projects",
        breadcrumbs: [
          { name: "/", path: "/" },
          { name: "home", path: "/home" },
          { name: "ora", path: "/home/ora" },
          { name: "projects", path: "/home/ora/projects" },
          { name: "ora-desktop", path: "/home/ora/projects/ora-desktop" },
        ],
        entries: [],
      },
      "/home/ora/projects/design-system": {
        currentPath: "/home/ora/projects/design-system",
        parentPath: "/home/ora/projects",
        breadcrumbs: [
          { name: "/", path: "/" },
          { name: "home", path: "/home" },
          { name: "ora", path: "/home/ora" },
          { name: "projects", path: "/home/ora/projects" },
          { name: "design-system", path: "/home/ora/projects/design-system" },
        ],
        entries: [],
      },
    },
  };
}

/** Owns the mutable arrays shared by every handler in one browser runtime. */
export const mockState = createInitialMockState();
