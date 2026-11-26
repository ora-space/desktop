import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import {
  agentRuntimeKeys,
  invalidateAgentCommands,
  invalidateAgentModels,
  refreshAgent,
} from "./agent-runtime";
import { invalidatePluginState } from "./plugin-lifecycle";
import { invalidatePluginQueries, pluginKeys } from "./plugins";
import { skillKeys } from "./skills";

/** Seed fresh caches so assertions observe actual prefix invalidation, not mocked calls. */
function caches() {
  const client = new QueryClient();
  const keys = {
    installed: pluginKeys.installedPlugins,
    available: pluginKeys.availablePlugins,
    skills: skillKeys.skills,
    availability: agentRuntimeKeys.agentRuntimeStatus,
    modelsA: agentRuntimeKeys.agentDiscovery(
      "agent-a",
      "workspace-1",
      "modelsAndCommands",
    ),
    modelsAElsewhere: agentRuntimeKeys.agentDiscovery(
      "agent-a",
      "workspace-2",
      "modelsAndCommands",
    ),
    modelsB: agentRuntimeKeys.agentDiscovery(
      "agent-b",
      "workspace-1",
      "modelsAndCommands",
    ),
  };
  for (const key of Object.values(keys)) client.setQueryData(key, []);
  return {
    client,
    invalidated: () =>
      Object.fromEntries(
        Object.entries(keys).map(([name, key]) => [
          name,
          client.getQueryState(key)?.isInvalidated,
        ]),
      ),
  };
}

describe("plugin lifecycle cache ownership", () => {
  it("refreshes all workspaces of the started agent, but never another agent", async () => {
    const { client, invalidated } = caches();
    await refreshAgent(client, "agent-a", "models");
    expect(invalidated()).toEqual({
      installed: false,
      available: false,
      skills: false,
      availability: true,
      modelsA: true,
      modelsAElsewhere: true,
      modelsB: false,
    });
    client.clear();
  });

  it("refreshes only the commands of one agent when Skills change", async () => {
    const { client, invalidated } = caches();
    const models = agentRuntimeKeys.agentDiscovery(
      "agent-a",
      "workspace-1",
      "models",
    );
    client.setQueryData(models, { models: [] });
    await invalidateAgentCommands(client, "agent-a");
    expect(invalidated()).toEqual({
      installed: false,
      available: false,
      skills: false,
      availability: false,
      modelsA: true,
      modelsAElsewhere: true,
      modelsB: false,
    });
    expect(client.getQueryState(models)?.isInvalidated).toBe(false);
    client.clear();
  });

  it.each([
    ["has none of its own", undefined, "chat"],
    ["has older ones", 1, "chat"],
    ["has newer ones", 3, "editor"],
  ])(
    "keeps the models discovered with outdated commands for a caller that %s",
    async (_case, ownUpdatedAt, expected) => {
      const client = new QueryClient();
      const key = (scope: "models" | "modelsAndCommands") =>
        agentRuntimeKeys.agentDiscovery("agent-a", "workspace-1", scope);
      const answer = (id: string) => ({ models: [{ id, displayName: id }] });
      client.setQueryData(
        key("modelsAndCommands"),
        { ...answer("chat"), commands: [] },
        { updatedAt: 2 },
      );
      if (ownUpdatedAt !== undefined)
        client.setQueryData(key("models"), answer("editor"), {
          updatedAt: ownUpdatedAt,
        });

      await invalidateAgentCommands(client, "agent-a");
      // A second change finds nothing that was still good.
      await invalidateAgentCommands(client, "agent-a");

      expect(client.getQueryData(key("models"))).toEqual(answer(expected));
      expect(client.getQueryState(key("models"))).toMatchObject({
        dataUpdatedAt: Math.max(2, ownUpdatedAt ?? 0),
        isInvalidated: false,
      });
      client.clear();
    },
  );

  it("shows no commands from before a Skill change, while refreshing or after a failed refresh", async () => {
    const client = new QueryClient();
    const queryKey = agentRuntimeKeys.agentDiscovery(
      "agent-a",
      "workspace-1",
      "modelsAndCommands",
    );
    const before = { models: [{ id: "m", displayName: "m" }], commands: [] };
    const refreshes: ((error: Error) => void)[] = [];
    const observer = new QueryObserver(client, {
      queryKey,
      queryFn: () =>
        refreshes.length === 0 && client.getQueryData(queryKey) === undefined
          ? Promise.resolve(before)
          : new Promise<object>((_resolve, reject) => {
              refreshes.push(reject);
            }),
      retry: false,
      staleTime: 5 * 60 * 1000,
    });
    const leave = observer.subscribe(() => {});
    await observer.refetch();
    const discoveredAt = client.getQueryState(queryKey)?.dataUpdatedAt;
    expect(observer.getCurrentResult().data).toEqual(before);

    const refreshed = invalidateAgentCommands(client, "agent-a");
    expect(refreshes).toHaveLength(1);
    const refreshing = { models: before.models };
    expect(observer.getCurrentResult().data).toEqual(refreshing);
    expect(client.getQueryState(queryKey)?.dataUpdatedAt).toBe(discoveredAt);

    refreshes[0]?.(new Error("agent unavailable"));
    await refreshed;
    expect(observer.getCurrentResult()).toMatchObject({
      data: refreshing,
      status: "error",
    });
    leave();
    client.clear();
  });

  it("shows no commands from before a Skill change that interrupts a running refresh", async () => {
    const client = new QueryClient();
    const queryKey = agentRuntimeKeys.agentDiscovery(
      "agent-a",
      "workspace-1",
      "modelsAndCommands",
    );
    const before = { models: [{ id: "m", displayName: "m" }], commands: [] };
    client.setQueryData(queryKey, before);
    const observer = new QueryObserver(client, {
      queryKey,
      queryFn: () => new Promise<object>(() => {}),
      retry: false,
      staleTime: 5 * 60 * 1000,
    });
    const leave = observer.subscribe(() => {});
    void observer.refetch();
    expect(client.getQueryState(queryKey)?.fetchStatus).toBe("fetching");

    void invalidateAgentCommands(client, "agent-a");

    expect(observer.getCurrentResult().data).toEqual({
      models: before.models,
    });
    leave();
    client.clear();
  });

  it("keeps no models that the agent itself has outdated", async () => {
    const { client } = caches();
    await invalidateAgentModels(client, "agent-a");

    await invalidateAgentCommands(client, "agent-a");

    expect(
      client.getQueryData(
        agentRuntimeKeys.agentDiscovery("agent-a", "workspace-1", "models"),
      ),
    ).toBeUndefined();
    client.clear();
  });

  it.each([
    ["a surface is showing it", true],
    ["no surface is showing it any more", false],
  ])(
    "drops a first discovery that answers after the Skills changed while %s",
    async (_case, isShown) => {
      const client = new QueryClient();
      const answers: ((answer: object) => void)[] = [];
      const queryKey = agentRuntimeKeys.agentDiscovery(
        "agent-a",
        "workspace-1",
        "modelsAndCommands",
      );
      const observer = new QueryObserver(client, {
        queryKey,
        queryFn: () =>
          new Promise<object>((resolve) => {
            answers.push(resolve);
          }),
        staleTime: 5 * 60 * 1000,
      });
      const leave = observer.subscribe(() => {});
      if (!isShown) leave();

      const refreshed = invalidateAgentCommands(client, "agent-a");
      answers[0]?.({ models: [], commands: ["before"] });
      await Promise.resolve();
      answers[1]?.({ models: [], commands: ["after"] });
      await refreshed;

      expect(answers).toHaveLength(isShown ? 2 : 1);
      expect(client.getQueryData(queryKey)).toEqual(
        isShown ? { models: [], commands: ["after"] } : undefined,
      );
      expect(
        client.getQueryData(
          agentRuntimeKeys.agentDiscovery("agent-a", "workspace-1", "models"),
        ),
      ).toBeUndefined();
      leave();
      client.clear();
    },
  );

  it.each([
    ["a surface is showing it", true],
    ["no surface is showing it any more", false],
  ])(
    "drops a first discovery that answers after the agent changed while %s",
    async (_case, isShown) => {
      const client = new QueryClient();
      const answers: ((answer: string) => void)[] = [];
      const queryKey = agentRuntimeKeys.agentDiscovery(
        "agent-a",
        "workspace-1",
        "modelsAndCommands",
      );
      const observer = new QueryObserver(client, {
        queryKey,
        queryFn: () =>
          new Promise<string>((resolve) => {
            answers.push(resolve);
          }),
        staleTime: 5 * 60 * 1000,
      });
      const leave = observer.subscribe(() => {});
      if (!isShown) leave();

      const refreshed = invalidateAgentModels(client, "agent-a");
      answers[0]?.("before");
      await Promise.resolve();
      answers[1]?.("after");
      await refreshed;

      expect(answers).toHaveLength(isShown ? 2 : 1);
      expect(client.getQueryData(queryKey)).toBe(isShown ? "after" : undefined);
      leave();
      client.clear();
    },
  );

  it("does not probe model discovery when an agent stops or is removed", async () => {
    const { client, invalidated } = caches();
    await refreshAgent(client, "agent-a", "availability");
    expect(invalidated()).toEqual({
      installed: false,
      available: false,
      skills: false,
      availability: true,
      modelsA: false,
      modelsAElsewhere: false,
      modelsB: false,
    });
    client.clear();
  });

  it("distinguishes external runtime events from completed catalog mutations", async () => {
    const event = caches();
    invalidatePluginState(event.client);
    expect(event.invalidated()).toEqual({
      installed: true,
      available: false,
      skills: true,
      availability: true,
      modelsA: false,
      modelsAElsewhere: false,
      modelsB: false,
    });
    event.client.clear();

    const mutation = caches();
    await invalidatePluginQueries(mutation.client);
    expect(mutation.invalidated()).toEqual({
      installed: true,
      available: true,
      skills: false,
      availability: false,
      modelsA: false,
      modelsAElsewhere: false,
      modelsB: false,
    });
    mutation.client.clear();
  });
});
