import assert from "node:assert/strict";
import { after, before, test } from "node:test";

import {
  ContractTransportError,
  createContractsClient,
  endpoints,
  type ContractsClient,
} from "@ora/contracts";
import { createFetchTransport } from "@ora/contracts/fetch";
import { setupServer } from "msw/node";
import { createMockHandlers } from "../src/handlers.js";
import { createInitialMockState } from "../src/state.js";

const initialNow = 1_700_000_000_000;
const state = createInitialMockState(initialNow);
const handlers = createMockHandlers(state);
const server = setupServer(...handlers);
let client: ContractsClient;

before(() => {
  server.listen({ onUnhandledRequest: "error" });
  client = createContractsClient(createFetchTransport({ baseUrl: "http://localhost" }));
});
after(() => server.close());

test("defines one Service Worker handler for every contracts endpoint", () => {
  assert.equal(handlers.length, Object.keys(endpoints).length);
  assert.deepEqual(Object.keys(endpoints).sort(), [
    "createProject",
    "createSession",
    "createTask",
    "deleteProject",
    "deleteSession",
    "deleteTask",
    "getProject",
    "getSession",
    "getTask",
    "listProjects",
    "listSessions",
    "listTasks",
    "openProjectWorkContext",
    "renewProjectWorkContext",
    "updateProject",
    "updateSession",
    "updateTask",
  ]);
});

test("starts every entity collection with representative in-memory data", async () => {
  const [projects, tasks, sessions] = await Promise.all([
    client.listProjects({}),
    client.listTasks({}),
    client.listSessions({}),
  ]);

  assert.deepEqual(projects, { projects: state.projects });
  assert.deepEqual(tasks, { tasks: state.tasks });
  assert.deepEqual(sessions, { sessions: state.sessions });
  assert.deepEqual(state.projectWorkContexts, [
    {
      id: "project-work-context-web",
      surface: "web",
      windowId: "prototype-window",
      projectId: "project-ora-desktop",
      leaseExpiresAt: initialNow + 120_000,
    },
  ]);
});

test("supports project create, get, update, and delete within one runtime", async () => {
  const created = await client.createProject({
    name: "Mock Service",
    rootPath: "C:\\workspace\\mock-service",
  });
  assert.match(created.project.id, /^project-/);
  assert.deepEqual(await client.getProject({ projectId: created.project.id }), created);

  const updated = await client.updateProject({
    projectId: created.project.id,
    name: "Mock Service Package",
    rootPath: "C:\\workspace\\ora\\packages\\mock-service",
  });
  assert.deepEqual(updated, {
    project: {
      id: created.project.id,
      name: "Mock Service Package",
      rootPath: "C:\\workspace\\ora\\packages\\mock-service",
    },
  });
  assert.deepEqual(await client.deleteProject({ projectId: created.project.id }), {
    projectId: created.project.id,
  });
  await assertNotFound(
    client.getProject({ projectId: created.project.id }),
    "project_not_found",
  );
});

test("supports task create, get, update, and delete within one runtime", async () => {
  const created = await client.createTask({
    projectId: "project-ora-desktop",
    title: "Cover every task endpoint",
    status: "todo",
  });
  assert.match(created.task.id, /^task-/);
  assert.deepEqual(await client.getTask({ taskId: created.task.id }), created);

  const updated = await client.updateTask({
    taskId: created.task.id,
    projectId: created.task.projectId,
    title: "Every task endpoint is covered",
    status: "done",
  });
  assert.deepEqual(updated, {
    task: {
      ...created.task,
      title: "Every task endpoint is covered",
      status: "done",
    },
  });
  assert.deepEqual(await client.deleteTask({ taskId: created.task.id }), {
    taskId: created.task.id,
  });
  await assertNotFound(client.getTask({ taskId: created.task.id }), "task_not_found");
});

test("supports session create, get, update, and delete within one runtime", async () => {
  const created = await client.createSession({
    taskId: "task-agent-runtime",
    agentId: "codex",
    agentSessionId: null,
    status: "running",
  });
  assert.match(created.session.id, /^session-/);
  assert.deepEqual(await client.getSession({ sessionId: created.session.id }), created);

  const updated = await client.updateSession({
    sessionId: created.session.id,
    taskId: created.session.taskId,
    agentId: created.session.agentId,
    agentSessionId: "remote-session-1",
    status: "stopped",
  });
  assert.deepEqual(updated, {
    session: {
      ...created.session,
      agentSessionId: "remote-session-1",
      status: "stopped",
    },
  });
  assert.deepEqual(await client.deleteSession({ sessionId: created.session.id }), {
    sessionId: created.session.id,
  });
  await assertNotFound(
    client.getSession({ sessionId: created.session.id }),
    "session_not_found",
  );
});

test("opens, switches, and renews project work contexts in memory", async () => {
  const opened = await client.openProjectWorkContext({
    surface: "web",
    windowId: "test-window",
    projectId: "project-ora-desktop",
  });
  assert.equal(opened.context.windowId, "test-window");
  assert.equal(typeof opened.context.leaseExpiresAt, "number");

  const switched = await client.openProjectWorkContext({
    surface: "web",
    windowId: "test-window",
    projectId: "project-design-system",
  });
  assert.equal(switched.context.id, opened.context.id);
  assert.equal(switched.context.projectId, "project-design-system");

  const renewed = await client.renewProjectWorkContext({
    surface: "web",
    windowId: "test-window",
  });
  assert.equal(renewed.context.id, opened.context.id);
  assert.equal(renewed.context.projectId, "project-design-system");
});

test("matches backend work-context conflict and not-found errors", async () => {
  await client.openProjectWorkContext({
    surface: "tauri",
    windowId: "tauri-window-1",
    projectId: "project-ora-desktop",
  });
  await assertTransportError(
    client.openProjectWorkContext({
      surface: "tauri",
      windowId: "tauri-window-2",
      projectId: "project-ora-desktop",
    }),
    "project_occupied",
    409,
  );
  await assertNotFound(
    client.renewProjectWorkContext({
      surface: "web",
      windowId: "missing-window",
    }),
    "project_work_context_not_found",
  );
  await assertNotFound(
    client.openProjectWorkContext({
      surface: "web",
      windowId: "another-window",
      projectId: "missing-project",
    }),
    "project_not_found",
  );
});

/** Verifies a rejected request uses the shared 404 transport error shape. */
async function assertNotFound(promise: Promise<unknown>, code: string): Promise<void> {
  await assertTransportError(promise, code, 404);
}

/** Verifies a rejected request preserves the structured contract error metadata. */
async function assertTransportError(
  promise: Promise<unknown>,
  code: string,
  status: number,
): Promise<void> {
  await assert.rejects(promise, (error: unknown) => {
    assert.ok(error instanceof ContractTransportError);
    assert.equal(error.code, code);
    assert.equal(error.status, status);

    return true;
  });
}
