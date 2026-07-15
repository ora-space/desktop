import type {
  CreateProjectRequest,
  CreateSessionRequest,
  CreateTaskRequest,
  EndpointOperation,
  OpenProjectWorkContextRequest,
  ProjectWorkContext,
  RenewProjectWorkContextRequest,
  UpdateProjectRequest,
  UpdateSessionRequest,
  UpdateTaskRequest,
} from "@ora/contracts";
import { HttpResponse, http, type HttpHandler } from "msw";
import { mockState, type MockState } from "./state.js";

const PROJECT_WORK_CONTEXT_LEASE_DURATION_MS = 120_000;

/** Produces a stable, readable identifier for newly created mock entities. */
function createId(prefix: string): string {
  return `${prefix}-${crypto.randomUUID()}`;
}

/** Returns the same structured error envelope as the HTTP adapter. */
function errorResponse(code: string, message: string, status: number) {
  return HttpResponse.json({ error: { code, message } }, { status });
}

/** Creates the complete HTTP handler set over a supplied in-memory state. */
export function createMockHandlers(state: MockState = mockState): HttpHandler[] {
  const handlersByOperation = {
    createProject: http.post("*/api/projects", async ({ request }) => {
      const body = (await request.json()) as CreateProjectRequest;
      const project = {
        id: createId("project"),
        name: body.name,
        rootPath: body.rootPath,
      };
      state.projects.push(project);

      return HttpResponse.json({ project }, { status: 201 });
    }),

    getProject: http.get("*/api/projects/:projectId", ({ params }) => {
      const projectId = String(params.projectId);
      const project = state.projects.find((candidate) => candidate.id === projectId);
      if (project === undefined) {
        return errorResponse("project_not_found", `project not found: ${projectId}`, 404);
      }

      return HttpResponse.json({ project });
    }),

    listProjects: http.get("*/api/projects", () => {
      return HttpResponse.json({ projects: state.projects });
    }),

    updateProject: http.put("*/api/projects/:projectId", async ({ params, request }) => {
      const projectId = String(params.projectId);
      const projectIndex = state.projects.findIndex((candidate) => candidate.id === projectId);
      if (projectIndex === -1) {
        return errorResponse("project_not_found", `project not found: ${projectId}`, 404);
      }

      const body = (await request.json()) as Omit<UpdateProjectRequest, "projectId">;
      const project = { id: projectId, name: body.name, rootPath: body.rootPath };
      state.projects[projectIndex] = project;

      return HttpResponse.json({ project });
    }),

    deleteProject: http.delete("*/api/projects/:projectId", ({ params }) => {
      const projectId = String(params.projectId);
      const projectIndex = state.projects.findIndex((candidate) => candidate.id === projectId);
      if (projectIndex === -1) {
        return errorResponse("project_not_found", `project not found: ${projectId}`, 404);
      }

      state.projects.splice(projectIndex, 1);

      return HttpResponse.json({ projectId });
    }),

    openProjectWorkContext: http.post("*/api/project-work-contexts/open", async ({ request }) => {
      const body = (await request.json()) as OpenProjectWorkContextRequest;
      const projectExists = state.projects.some((project) => project.id === body.projectId);
      if (!projectExists) {
        return errorResponse(
          "project_not_found",
          `project not found: ${body.projectId}`,
          404,
        );
      }

      const now = Date.now();
      const existingIndex = state.projectWorkContexts.findIndex(
        (context) => context.surface === body.surface && context.windowId === body.windowId,
      );
      const conflictingContext = state.projectWorkContexts.find(
        (context) =>
          context.projectId === body.projectId &&
          context.surface === "tauri" &&
          context.leaseExpiresAt > now &&
          context.id !== state.projectWorkContexts[existingIndex]?.id,
      );
      if (body.surface === "tauri" && conflictingContext !== undefined) {
        return errorResponse(
          "project_occupied",
          `project is already occupied: ${body.projectId}`,
          409,
        );
      }

      const context: ProjectWorkContext = {
        id:
          existingIndex === -1
            ? createId("project-work-context")
            : state.projectWorkContexts[existingIndex]!.id,
        surface: body.surface,
        windowId: body.windowId,
        projectId: body.projectId,
        leaseExpiresAt: now + PROJECT_WORK_CONTEXT_LEASE_DURATION_MS,
      };
      if (existingIndex === -1) {
        state.projectWorkContexts.push(context);
      } else {
        state.projectWorkContexts[existingIndex] = context;
      }

      return HttpResponse.json({ context });
    }),

    renewProjectWorkContext: http.post("*/api/project-work-contexts/renew", async ({ request }) => {
      const body = (await request.json()) as RenewProjectWorkContextRequest;
      const contextIndex = state.projectWorkContexts.findIndex(
        (context) => context.surface === body.surface && context.windowId === body.windowId,
      );
      if (contextIndex === -1) {
        return errorResponse(
          "project_work_context_not_found",
          `project work context not found for ${body.surface}/${body.windowId}`,
          404,
        );
      }

      const context: ProjectWorkContext = {
        ...state.projectWorkContexts[contextIndex]!,
        leaseExpiresAt: Date.now() + PROJECT_WORK_CONTEXT_LEASE_DURATION_MS,
      };
      state.projectWorkContexts[contextIndex] = context;

      return HttpResponse.json({ context });
    }),

    createTask: http.post("*/api/tasks", async ({ request }) => {
      const body = (await request.json()) as CreateTaskRequest;
      const task = {
        id: createId("task"),
        projectId: body.projectId,
        title: body.title,
        status: body.status,
      };
      state.tasks.push(task);

      return HttpResponse.json({ task }, { status: 201 });
    }),

    getTask: http.get("*/api/tasks/:taskId", ({ params }) => {
      const taskId = String(params.taskId);
      const task = state.tasks.find((candidate) => candidate.id === taskId);
      if (task === undefined) {
        return errorResponse("task_not_found", `task not found: ${taskId}`, 404);
      }

      return HttpResponse.json({ task });
    }),

    listTasks: http.get("*/api/tasks", () => {
      return HttpResponse.json({ tasks: state.tasks });
    }),

    updateTask: http.put("*/api/tasks/:taskId", async ({ params, request }) => {
      const taskId = String(params.taskId);
      const taskIndex = state.tasks.findIndex((candidate) => candidate.id === taskId);
      if (taskIndex === -1) {
        return errorResponse("task_not_found", `task not found: ${taskId}`, 404);
      }

      const body = (await request.json()) as Omit<UpdateTaskRequest, "taskId">;
      const task = {
        id: taskId,
        projectId: body.projectId,
        title: body.title,
        status: body.status,
      };
      state.tasks[taskIndex] = task;

      return HttpResponse.json({ task });
    }),

    deleteTask: http.delete("*/api/tasks/:taskId", ({ params }) => {
      const taskId = String(params.taskId);
      const taskIndex = state.tasks.findIndex((candidate) => candidate.id === taskId);
      if (taskIndex === -1) {
        return errorResponse("task_not_found", `task not found: ${taskId}`, 404);
      }

      state.tasks.splice(taskIndex, 1);

      return HttpResponse.json({ taskId });
    }),

    createSession: http.post("*/api/sessions", async ({ request }) => {
      const body = (await request.json()) as CreateSessionRequest;
      const session = {
        id: createId("session"),
        taskId: body.taskId,
        agentId: body.agentId,
        agentSessionId: body.agentSessionId,
        status: body.status,
      };
      state.sessions.push(session);

      return HttpResponse.json({ session }, { status: 201 });
    }),

    getSession: http.get("*/api/sessions/:sessionId", ({ params }) => {
      const sessionId = String(params.sessionId);
      const session = state.sessions.find((candidate) => candidate.id === sessionId);
      if (session === undefined) {
        return errorResponse("session_not_found", `session not found: ${sessionId}`, 404);
      }

      return HttpResponse.json({ session });
    }),

    listSessions: http.get("*/api/sessions", () => {
      return HttpResponse.json({ sessions: state.sessions });
    }),

    updateSession: http.put("*/api/sessions/:sessionId", async ({ params, request }) => {
      const sessionId = String(params.sessionId);
      const sessionIndex = state.sessions.findIndex((candidate) => candidate.id === sessionId);
      if (sessionIndex === -1) {
        return errorResponse("session_not_found", `session not found: ${sessionId}`, 404);
      }

      const body = (await request.json()) as Omit<UpdateSessionRequest, "sessionId">;
      const session = {
        id: sessionId,
        taskId: body.taskId,
        agentId: body.agentId,
        agentSessionId: body.agentSessionId,
        status: body.status,
      };
      state.sessions[sessionIndex] = session;

      return HttpResponse.json({ session });
    }),

    deleteSession: http.delete("*/api/sessions/:sessionId", ({ params }) => {
      const sessionId = String(params.sessionId);
      const sessionIndex = state.sessions.findIndex((candidate) => candidate.id === sessionId);
      if (sessionIndex === -1) {
        return errorResponse("session_not_found", `session not found: ${sessionId}`, 404);
      }

      state.sessions.splice(sessionIndex, 1);

      return HttpResponse.json({ sessionId });
    }),
  } satisfies Record<EndpointOperation, HttpHandler>;

  return Object.values(handlersByOperation);
}

export const handlers = createMockHandlers();
