import { describe, it, expect } from "vitest";
import { waitFor } from "@testing-library/react";
import { useAgents } from "./use-agents";
import { useSkills } from "./use-skills";
import {
  useCreateAgent,
  useUpdateAgent,
  useDeleteAgent,
  useCreateSkill,
  useUpdateSkill,
  useDeleteSkill,
} from "./use-atom-mutations";
import {
  createMockClient,
  createMockClientState,
} from "../../test/mock-client";
import { renderHookWithClient } from "../../test/hook-harness";
import { queryKeys } from "./query-keys";
import type { Agent, Skill } from "@ora/contracts";

const AGENT_A: Agent = {
  id: "a1",
  namespace: "local",
  name: "Codex",
  description: "Code agent",
};
const SKILL_X: Skill = {
  id: "sk1",
  namespace: "local",
  name: "Refactor",
  description: "Refactoring skill",
  availability: "available",
};

describe("useAgents", () => {
  it("returns the agent list from the client", async () => {
    const state = createMockClientState();
    state.agents = [AGENT_A];
    const client = createMockClient(state);
    const { result } = renderHookWithClient(() => useAgents(), client);
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual([AGENT_A]);
  });
});

describe("useSkills", () => {
  it("returns the skill list from the client", async () => {
    const state = createMockClientState();
    state.skills = [SKILL_X];
    const client = createMockClient(state);
    const { result } = renderHookWithClient(() => useSkills(), client);
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual([SKILL_X]);
  });
});

describe("useCreateAgent", () => {
  it("creates an agent and invalidates the agent list", async () => {
    const state = createMockClientState();
    const client = createMockClient(state);
    const agents = renderHookWithClient(() => useAgents(), client);
    const mutation = renderHookWithClient(
      () => useCreateAgent(),
      client,
      agents.queryClient,
    );

    await waitFor(() => expect(agents.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({
      name: "New Agent",
      description: "desc",
      content: "# Agent",
    });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.agents).toHaveLength(1);
    expect(state.agents[0]!.name).toBe("New Agent");
    await agents.queryClient.refetchQueries({ queryKey: queryKeys.agents });
    expect(
      agents.queryClient.getQueryData<Agent[]>(queryKeys.agents),
    ).toHaveLength(1);
  });
});

describe("useUpdateAgent", () => {
  it("updates the agent fields", async () => {
    const state = createMockClientState();
    state.agents = [AGENT_A];
    const client = createMockClient(state);
    const agents = renderHookWithClient(() => useAgents(), client);
    const mutation = renderHookWithClient(
      () => useUpdateAgent(),
      client,
      agents.queryClient,
    );

    await waitFor(() => expect(agents.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({
      agent: AGENT_A,
      name: "Renamed",
      description: "new desc",
      content: "# Updated agent",
    });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.agents[0]).toEqual({
      id: "a1",
      namespace: "local",
      name: "Renamed",
      description: "new desc",
    });
  });
});

describe("useDeleteAgent", () => {
  it("deletes the agent", async () => {
    const state = createMockClientState();
    state.agents = [AGENT_A];
    const client = createMockClient(state);
    const agents = renderHookWithClient(() => useAgents(), client);
    const mutation = renderHookWithClient(
      () => useDeleteAgent(),
      client,
      agents.queryClient,
    );

    await waitFor(() => expect(agents.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({ agentId: "a1" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.agents).toEqual([]);
  });
});

describe("useCreateSkill", () => {
  it("creates a skill and invalidates the skill list", async () => {
    const state = createMockClientState();
    const client = createMockClient(state);
    const skills = renderHookWithClient(() => useSkills(), client);
    const mutation = renderHookWithClient(
      () => useCreateSkill(),
      client,
      skills.queryClient,
    );

    await waitFor(() => expect(skills.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({
      name: "New Skill",
      description: "desc",
      content: "# Skill",
    });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.skills).toHaveLength(1);
    expect(state.skills[0]!.name).toBe("New Skill");
    await skills.queryClient.refetchQueries({ queryKey: queryKeys.skills });
    expect(
      skills.queryClient.getQueryData<Skill[]>(queryKeys.skills),
    ).toHaveLength(1);
  });
});

describe("useUpdateSkill", () => {
  it("updates the skill fields", async () => {
    const state = createMockClientState();
    state.skills = [SKILL_X];
    const client = createMockClient(state);
    const skills = renderHookWithClient(() => useSkills(), client);
    const mutation = renderHookWithClient(
      () => useUpdateSkill(),
      client,
      skills.queryClient,
    );

    await waitFor(() => expect(skills.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({
      skill: SKILL_X,
      name: "Renamed",
      description: "new desc",
      content: "# Updated skill",
    });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.skills[0]).toEqual({
      id: "sk1",
      namespace: "local",
      name: "Renamed",
      description: "new desc",
      availability: "available",
    });
  });
});

describe("useDeleteSkill", () => {
  it("deletes the skill", async () => {
    const state = createMockClientState();
    state.skills = [SKILL_X];
    const client = createMockClient(state);
    const skills = renderHookWithClient(() => useSkills(), client);
    const mutation = renderHookWithClient(
      () => useDeleteSkill(),
      client,
      skills.queryClient,
    );

    await waitFor(() => expect(skills.result.current.isSuccess).toBe(true));
    mutation.result.current.mutate({ skillId: "sk1" });
    await waitFor(() => expect(mutation.result.current.isSuccess).toBe(true));

    expect(state.skills).toEqual([]);
  });
});
