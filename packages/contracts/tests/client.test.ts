import assert from "node:assert/strict";
import test from "node:test";

import { createContractsClient } from "../src/client.js";
import { endpoints } from "../src/endpoints.js";
import type { ContractTransport, ContractTransportRequest } from "../src/transport.js";

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
    projectId: "project-1",
    title: "Ship SDK",
    status: "doing",
  });

  assert.deepEqual(requests, [
    {
      operationName: "updateTask",
      method: "PUT",
      path: "/api/tasks/task-1",
      body: {
        projectId: "project-1",
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
