import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import { agentRuntimeKeys, refreshAgent } from "./agent-runtime";
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
    modelsA: agentRuntimeKeys.agentModels("agent-a", "workspace-1"),
    modelsAElsewhere: agentRuntimeKeys.agentModels("agent-a", "workspace-2"),
    modelsB: agentRuntimeKeys.agentModels("agent-b", "workspace-1"),
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
