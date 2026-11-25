import { QueryClient, QueryObserver } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import {
  agentRuntimeKeys,
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
