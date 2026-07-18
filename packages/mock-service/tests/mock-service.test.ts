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
    "createAgent",
    "createProject",
    "createSession",
    "createSkill",
    "createTask",
    "deleteAgent",
    "deleteProject",
    "deleteSession",
    "deleteSkill",
    "deleteTask",
    "getAgent",
    "getProject",
    "getSession",
    "getSkill",
    "getTask",
    "listAgents",
    "listDirectory",
    "listProjects",
    "listSessions",
    "listSkills",
    "listTasks",
    "openProjectWorkContext",
    "renewProjectWorkContext",
    "updateAgent",
    "updateProject",
    "updateSession",
    "updateSkill",
    "updateTask",
  ]);
});

test("lists deterministic mock filesystem directories from home or an explicit query path", async () => {
  assert.deepEqual(await client.fileSystem.listDirectory({}), state.fileSystemDirectories[state.homeDirectory]);
  assert.deepEqual(
    await client.fileSystem.listDirectory({ path: "/home/ora/projects" }),
    state.fileSystemDirectories["/home/ora/projects"],
  );
  for (const project of state.projects) {
    assert.ok(state.fileSystemDirectories[project.rootPath]);
  }
});

test("starts every entity collection with representative in-memory data", async () => {
  const [projects, tasks, sessions, skills, agents] = await Promise.all([
    client.project.list({}),
    client.task.list({}),
    client.session.list({}),
    client.skill.list({}),
    client.agent.list({}),
  ]);

  assert.deepEqual(projects, { projects: state.projects });
  assert.deepEqual(tasks, { tasks: state.tasks });
  assert.deepEqual(sessions, { sessions: state.sessions });
  assert.deepEqual(skills, { skills: state.skills });
  assert.deepEqual(agents, { agents: state.agents });
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

test("supports skill CRUD within one runtime", async () => {
  const created = await client.skill.create({ name: "testing", description: "Runs focused tests." });
  assert.match(created.skill.id, /^skill-/);
  assert.deepEqual(await client.skill.get({ skillId: created.skill.id }), created);

  const updated = await client.skill.update({
    skillId: created.skill.id,
    name: "test-runner",
    description: "Runs the relevant test suite.",
  });
  assert.equal(updated.skill.name, "test-runner");
  assert.deepEqual(await client.skill.delete({ skillId: created.skill.id }), { skillId: created.skill.id });
  await assertNotFound(client.skill.get({ skillId: created.skill.id }), "skill_not_found");
});

test("supports agent CRUD within one runtime", async () => {
  const created = await client.agent.create({ name: "Reviewer", description: "Reviews a change." });
  assert.match(created.agent.id, /^agent-/);
  assert.deepEqual(await client.agent.get({ agentId: created.agent.id }), created);

  const updated = await client.agent.update({
    agentId: created.agent.id,
    name: "Code Reviewer",
    description: "Reviews code and tests.",
  });
  assert.equal(updated.agent.name, "Code Reviewer");
  assert.deepEqual(await client.agent.delete({ agentId: created.agent.id }), { agentId: created.agent.id });
  await assertNotFound(client.agent.get({ agentId: created.agent.id }), "agent_not_found");
});

test("supports project create, get, update, and delete within one runtime", async () => {
  const created = await client.project.create({
    name: "Mock Service",
    rootPath: "C:\\workspace\\mock-service",
  });
  assert.match(created.project.id, /^project-/);
  assert.deepEqual(await client.project.get({ projectId: created.project.id }), created);

  const updated = await client.project.update({
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
  assert.deepEqual(await client.project.delete({ projectId: created.project.id }), {
    projectId: created.project.id,
  });
  await assertNotFound(
    client.project.get({ projectId: created.project.id }),
    "project_not_found",
  );
});

test("supports task create, get, update, and delete within one runtime", async () => {
  const created = await client.task.create({
    projectId: "project-ora-desktop",
    title: "Cover every task endpoint",
    status: "todo",
  });
  assert.match(created.task.id, /^task-/);
  assert.deepEqual(await client.task.get({ taskId: created.task.id }), created);

  const updated = await client.task.update({
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
  assert.deepEqual(await client.task.delete({ taskId: created.task.id }), {
    taskId: created.task.id,
  });
  await assertNotFound(client.task.get({ taskId: created.task.id }), "task_not_found");
});

test("supports session create, get, update, and delete within one runtime", async () => {
  const created = await client.session.create({
    taskId: "task-agent-runtime",
    agentId: "codex",
    agentSessionId: null,
    status: "running",
  });
  assert.match(created.session.id, /^session-/);
  assert.deepEqual(await client.session.get({ sessionId: created.session.id }), created);

  const updated = await client.session.update({
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
  assert.deepEqual(await client.session.delete({ sessionId: created.session.id }), {
    sessionId: created.session.id,
  });
  await assertNotFound(
    client.session.get({ sessionId: created.session.id }),
    "session_not_found",
  );
});

test("opens, switches, and renews project work contexts in memory", async () => {
  const opened = await client.projectWorkContext.open({
    surface: "web",
    windowId: "test-window",
    projectId: "project-ora-desktop",
  });
  assert.equal(opened.context.windowId, "test-window");
  assert.equal(typeof opened.context.leaseExpiresAt, "number");

  const switched = await client.projectWorkContext.open({
    surface: "web",
    windowId: "test-window",
    projectId: "project-design-system",
  });
  assert.equal(switched.context.id, opened.context.id);
  assert.equal(switched.context.projectId, "project-design-system");

  const renewed = await client.projectWorkContext.renew({
    surface: "web",
    windowId: "test-window",
  });
  assert.equal(renewed.context.id, opened.context.id);
  assert.equal(renewed.context.projectId, "project-design-system");
});

test("matches backend work-context conflict and not-found errors", async () => {
  await client.projectWorkContext.open({
    surface: "tauri",
    windowId: "tauri-window-1",
    projectId: "project-ora-desktop",
  });
  await assertTransportError(
    client.projectWorkContext.open({
      surface: "tauri",
      windowId: "tauri-window-2",
      projectId: "project-ora-desktop",
    }),
    "project_occupied",
    409,
  );
  await assertNotFound(
    client.projectWorkContext.renew({
      surface: "web",
      windowId: "missing-window",
    }),
    "project_work_context_not_found",
  );
  await assertNotFound(
    client.projectWorkContext.open({
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
