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
