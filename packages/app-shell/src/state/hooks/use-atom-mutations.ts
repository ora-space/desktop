import { useMutation, useQueryClient } from "@tanstack/react-query";
import type { Agent, Skill } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Creates a configurable agent and refreshes the agent list. */
export function useCreateAgent() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, description, content }: { name: string; description: string; content: string }) =>
      client.agent.create({ name, description, content }).then((response) => response.agent),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents });
    },
  });
}

/** Replaces a configurable agent's fields and refreshes the agent list. */
export function useUpdateAgent() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ agent, name, description, content }: { agent: Agent; name: string; description: string; content: string }) =>
      client.agent.update({ agentId: agent.id, name, description, content }).then((response) => response.agent),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents });
    },
  });
}

/** Soft-deletes a configurable agent and refreshes the agent list. */
export function useDeleteAgent() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ agentId }: { agentId: string }) =>
      client.agent.delete({ agentId }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.agents });
    },
  });
}

/** Creates a skill and refreshes the skill list. */
export function useCreateSkill() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ name, description, content }: { name: string; description: string; content: string }) =>
      client.skill.create({ name, description, content }).then((response) => response.skill),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.skills });
    },
  });
}

/** Replaces a skill's fields and refreshes the skill list. */
export function useUpdateSkill() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ skill, name, description, content }: { skill: Skill; name: string; description: string; content: string }) =>
      client.skill.update({ skillId: skill.id, name, description, content }).then((response) => response.skill),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.skills });
    },
  });
}

/** Soft-deletes a skill and refreshes the skill list. */
export function useDeleteSkill() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: ({ skillId }: { skillId: string }) =>
      client.skill.delete({ skillId }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: queryKeys.skills });
    },
  });
}
