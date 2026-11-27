import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import type { Skill } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { skillKeys } from "../data/skills";

const NO_RETIRED_SKILLS: readonly string[] = [];

/** Pickers only surface skills whose on-disk package is still usable. */
export function availableSkills(skills: readonly Skill[]): Skill[] {
  return skills.filter((skill) => skill.availability === "available");
}

/** Shares the complete response while keeping existing picker consumers on visible entries. */
export function useSkills(enabled = true) {
  const client = useContractsClient();
  const query = useQuery({
    queryKey: skillKeys.skills,
    queryFn: () => client.skill.list({}),
    enabled,
  });
  const localSkillNamesForExport = useMemo(
    () =>
      query.data?.localSkillNamesForExport ??
      Object.fromEntries(
        (query.data?.skills ?? [])
          .filter((skill) => skill.source.kind === "local")
          .map((skill) => [skill.id, skill.name]),
      ),
    [query.data],
  );
  return {
    ...query,
    data: query.data?.skills,
    retiredSkillIds: query.data?.retiredSkillIds ?? NO_RETIRED_SKILLS,
    localSkillNamesForExport,
  };
}
