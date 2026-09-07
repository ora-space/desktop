import type { QueryClient } from "@tanstack/react-query";

/** Cache identity owned by skills data; consumers never repeat its tuples. */
export const skillKeys = {
  skills: ["skills"] as const,
};

/** Package lifecycle changes can add or remove installed skill projections. */
export function invalidateSkills(queryClient: QueryClient) {
  return queryClient.invalidateQueries({ queryKey: skillKeys.skills });
}
