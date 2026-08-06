import assert from "node:assert/strict";
import test from "node:test";

import {
  createMemoryContractsClient,
  createMemoryContractsState,
} from "../src/memory.js";

test("serves isolated seeded state without exposing mutable records", async () => {
  const state = createMemoryContractsState({
    projects: [{ id: "p1", name: "Prototype", rootPath: "/workspace/prototype" }],
  });
  const client = createMemoryContractsClient(state);

  const first = await client.project.list({});
  first.projects[0]!.name = "mutated outside";

  assert.deepEqual(await client.project.list({}), {
    projects: [{ id: "p1", name: "Prototype", rootPath: "/workspace/prototype" }],
  });
});

test("keeps CRUD stateful and avoids reusing an existing identifier", async () => {
  const state = createMemoryContractsState({
    projects: [
      { id: "p1", name: "One", rootPath: "/one" },
      { id: "p3", name: "Three", rootPath: "/three" },
    ],
  });
  const client = createMemoryContractsClient(state);

  const created = await client.project.create({ name: "Two", rootPath: "/two" });
  await client.project.delete({ projectId: "p1" });

  assert.equal(created.project.id, "p2");
  assert.deepEqual(
    (await client.project.list({})).projects.map((project) => project.id),
    ["p3", "p2"],
  );
});

test("persists Agent and Skill content in memory detail responses", async () => {
  const client = createMemoryContractsClient();
  const createdAgent = await client.agent.create({
    name: "reviewer",
    description: "Reviews changes",
    content: "agent body",
  });
  const createdSkill = await client.skill.create({
    name: "review-skill",
    description: "Reviews changes",
    content: "skill body",
  });

  assert.equal((await client.agent.get({ agentId: createdAgent.agent.id })).agent.content, "agent body");
  assert.equal((await client.skill.get({ skillId: createdSkill.skill.id })).skill.content, "skill body");

  await client.agent.update({
    agentId: createdAgent.agent.id,
    name: "reviewer",
    description: "Reviews carefully",
    content: "updated agent body",
  });
  await client.skill.update({
    skillId: createdSkill.skill.id,
    name: "review-skill",
    description: "Reviews carefully",
    content: "updated skill body",
  });

  assert.equal((await client.agent.get({ agentId: createdAgent.agent.id })).agent.content, "updated agent body");
  assert.equal((await client.skill.get({ skillId: createdSkill.skill.id })).skill.content, "updated skill body");
});

test("opens and renews a project work-context lease in memory", async () => {
  const client = createMemoryContractsClient(createMemoryContractsState({
    projects: [{ id: "p1", name: "Prototype", rootPath: "/workspace/prototype" }],
  }));

  const opened = await client.projectWorkContext.open({
    surface: "web",
    windowId: "window-1",
    projectId: "p1",
  });
  const renewed = await client.projectWorkContext.renew({
    surface: "web",
    windowId: "window-1",
  });

  assert.deepEqual(renewed.context, {
    ...opened.context,
    leaseExpiresAt: renewed.context.leaseExpiresAt,
  });
  assert.ok(renewed.context.leaseExpiresAt >= opened.context.leaseExpiresAt);
});

test("provides safe empty specification data for prototype clients", async () => {
  const client = createMemoryContractsClient();

  assert.deepEqual(
    await client.spec.catalog({ target: { kind: "project", projectId: "p1" } }),
    { sources: [], documents: [], truncated: false },
  );
  assert.deepEqual(
    await client.spec.updateProjectSources({
      projectId: "p1",
      sources: [{
        relativePath: "docs/specs",
        workflow: { kind: "open_spec" },
        visibility: "enabled",
      }],
    }),
    {
      sources: [{
        relativePath: "docs/specs",
        workflow: { kind: "open_spec" },
        visibility: "enabled",
      }],
    },
  );
});
