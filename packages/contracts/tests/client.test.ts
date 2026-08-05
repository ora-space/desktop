import assert from "node:assert/strict";
import test from "node:test";

import { createContractsClient } from "../src/client.js";
import { endpoints } from "../src/endpoints.js";
import type {
  ContractCallOptions,
  ContractTransport,
  ContractTransportRequest,
} from "../src/transport.js";

/**
 * Builds a transport double that records requests and returns a fixed response.
 */
function recordingTransport<TResponse>(
  requests: ContractTransportRequest[],
  response: TResponse,
): ContractTransport {
  return {
    async send<TTransportResponse>(
      request: ContractTransportRequest,
    ): Promise<TTransportResponse> {
      requests.push(request);

      return response as unknown as TTransportResponse;
    },
    stream<TEvent>(): AsyncIterable<TEvent> {
      throw new Error("stream was not expected in this test");
    },
  };
}

test("builds update URLs from path params and JSON bodies", async () => {
  const requests: ContractTransportRequest[] = [];
  const client = createContractsClient(
    recordingTransport(requests, {
      task: {
        id: "task-1",
        projectId: "project-1",
        title: "Ship SDK",
        status: "doing",
      },
    }),
  );
  const response = await client.task.update({
    taskId: "task-1",
    title: "Ship SDK",
    status: "doing",
  });

  assert.deepEqual(requests, [
    {
      operationName: "updateTask",
      request: {
        taskId: "task-1",
        title: "Ship SDK",
        status: "doing",
      },
      method: "PUT",
      path: "/api/tasks/task-1",
      body: {
        title: "Ship SDK",
        status: "doing",
      },
      headers: {
        "content-type": "application/json",
      },
    },
  ]);
  assert.deepEqual(response, {
    task: {
      id: "task-1",
      projectId: "project-1",
      title: "Ship SDK",
      status: "doing",
    },
  });
});

test("omits JSON bodies for path-only operations", async () => {
  const requests: ContractTransportRequest[] = [];
  const client = createContractsClient(
    recordingTransport(requests, {
      project: {
        id: "project-1",
        name: "Ora",
        rootPath: "/workspace/ora",
      },
    }),
  );

  await client.project.get({
    projectId: "project-1",
  });

  assert.deepEqual(requests, [
    {
      operationName: "getProject",
      request: { projectId: "project-1" },
      method: "GET",
      path: "/api/projects/project-1",
      body: undefined,
      headers: {},
    },
  ]);
});

test("encodes optional query parameters without adding a JSON body", async () => {
  const requests: ContractTransportRequest[] = [];
  const client = createContractsClient(
    recordingTransport(requests, {
      currentPath: "/home/ora/projects & tools",
      parentPath: "/home/ora",
      breadcrumbs: [],
      entries: [],
    }),
  );

  await client.fileSystem.listDirectory({ path: "/home/ora/projects & tools" });

  assert.deepEqual(requests, [
    {
      operationName: "listDirectory",
      request: { path: "/home/ora/projects & tools" },
      method: "GET",
      path: "/api/file-system/directory?path=%2Fhome%2Fora%2Fprojects+%26+tools",
      body: undefined,
      headers: {},
    },
  ]);
});

test("omits absent optional query parameters", async () => {
  const requests: ContractTransportRequest[] = [];
  const client = createContractsClient(
    recordingTransport(requests, {
      currentPath: "/home/ora",
      parentPath: "/home",
      breadcrumbs: [],
      entries: [],
    }),
  );

  await client.fileSystem.listDirectory({});

  assert.deepEqual(requests[0]?.path, "/api/file-system/directory");
});

test("posts a warm session request against the static warm path", async () => {
  const requests: ContractTransportRequest[] = [];
  const client = createContractsClient(
    recordingTransport(requests, { sessionId: "session-1", configOptions: [] }),
  );
  const request = {
    target: { type: "projectRoot", projectId: "project-1" } as const,
    agentCli: "open_code" as const,
    clientId: "client-1",
  };

  await client.session.warm(request);

  assert.deepEqual(requests, [
    {
      operationName: "warmSession",
      request,
      method: "POST",
      path: "/api/sessions/warm",
      body: request,
      headers: { "content-type": "application/json" },
    },
  ]);
});

test("puts the session id in the config path while the choice stays in the body", async () => {
  const requests: ContractTransportRequest[] = [];
  const client = createContractsClient(recordingTransport(requests, { configOptions: [] }));
  const request = { sessionId: "session-1", configId: "model", value: "fast" };

  await client.session.setConfig(request);

  assert.deepEqual(requests, [
    {
      operationName: "setSessionConfig",
      request,
      method: "POST",
      path: "/api/sessions/session-1/config",
      // The session id addresses the route, so it is not repeated in the body.
      body: { configId: "model", value: "fast" },
      headers: { "content-type": "application/json" },
    },
  ]);
});

test("puts the session id in the commit path while decisions stay in JSON", async () => {
  const requests: ContractTransportRequest[] = [];
  const client = createContractsClient(
    recordingTransport(requests, {
      sessionId: "import-1",
      status: "committing",
      progress: { processed: 0, total: 1, results: [] },
    }),
  );
  const request = {
    sessionId: "import-1",
    decisions: [{ candidateId: "candidate-1", decision: "skip" as const }],
  };

  await client.skillImport.commit(request);

  assert.deepEqual(requests, [
    {
      operationName: "commitSkillImport",
      request,
      method: "POST",
      path: "/api/skill-imports/import-1/commit",
      body: {
        decisions: [{ candidateId: "candidate-1", decision: "skip" }],
      },
      headers: {
        "content-type": "application/json",
      },
    },
  ]);
});

test("uses a skill id in PUT paths while leaving editable fields in JSON", async () => {
  const requests: ContractTransportRequest[] = [];
  const client = createContractsClient(
    recordingTransport(requests, {
      skill: {
        id: "skill-1",
        name: "code-review",
        description: "Reviews code",
      },
    }),
  );

  await client.skill.update({
    skillId: "skill-1",
    name: "code-review",
    description: "Reviews code",
  });

  assert.deepEqual(requests, [
    {
      operationName: "updateSkill",
      request: {
        skillId: "skill-1",
        name: "code-review",
        description: "Reviews code",
      },
      method: "PUT",
      path: "/api/skills/skill-1",
      body: {
        name: "code-review",
        description: "Reviews code",
      },
      headers: {
        "content-type": "application/json",
      },
    },
  ]);
});

test("splits task diff comment identifiers from the review body", async () => {
  const requests: ContractTransportRequest[] = [];
  const client = createContractsClient(
    recordingTransport(requests, {
      comment: {
        id: "reply-1",
        taskId: "task-1",
        kind: { kind: "reply", parentCommentId: "comment-1" },
        body: "Updated.",
        createdAt: 1,
        updatedAt: 1,
      },
    }),
  );

  await client.task.replyDiffComment({
    taskId: "task-1",
    commentId: "comment-1",
    body: "Updated.",
  });

  assert.deepEqual(requests, [
    {
      operationName: "replyTaskDiffComment",
      request: {
        taskId: "task-1",
        commentId: "comment-1",
        body: "Updated.",
      },
      method: "POST",
      path: "/api/tasks/task-1/diff/comments/comment-1/replies",
      body: { body: "Updated." },
      headers: {
        "content-type": "application/json",
      },
    },
  ]);
});

test("omits standalone worktree operations from generated contracts", () => {
  assert.equal("createWorktree" in endpoints, false);
  assert.equal("getWorktree" in endpoints, false);
  assert.equal("listWorktrees" in endpoints, false);
  assert.equal("updateWorktree" in endpoints, false);
  assert.equal("deleteWorktree" in endpoints, false);

  const client = createContractsClient(
    recordingTransport([], {
      task: {
        id: "task-1",
        projectId: "project-1",
        title: "Ship SDK",
        status: "doing",
      },
    }),
  );

  assert.equal("createWorktree" in client, false);
  assert.equal("getWorktree" in client, false);
  assert.equal("listWorktrees" in client, false);
  assert.equal("updateWorktree" in client, false);
  assert.equal("deleteWorktree" in client, false);
});

test("forwards call options through every unary client operation", async () => {
  const controller = new AbortController();
  let observedSignal: AbortSignal | undefined;
  const transport: ContractTransport = {
    async send<TResponse>(
      _request: ContractTransportRequest,
      options?: ContractCallOptions,
    ): Promise<TResponse> {
      observedSignal = options?.signal;
      return { projects: [] } as TResponse;
    },
    stream<TEvent>(): AsyncIterable<TEvent> {
      throw new Error("stream was not expected in this test");
    },
  };
  const client = createContractsClient(transport);

  await client.project.list({}, { signal: controller.signal });

  assert.equal(observedSignal, controller.signal);
});
