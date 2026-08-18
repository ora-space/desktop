import { useQuery } from "@tanstack/react-query";
import type { Skill } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Pickers only surface skills whose on-disk package is still usable. */
export function availableSkills(skills: readonly Skill[]): Skill[] {
  return skills.filter((skill) => skill.availability === "available");
}

/** Loads configurable skills through the contracts client and caches them. */
export function useSkills() {
  const client = useContractsClient();
  return useQuery({
    queryKey: queryKeys.skills,
    queryFn: () => client.skill.list({}).then((response) => response.skills),
  });
}
