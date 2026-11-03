import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { retryEffectTarget } from "../data/effects";

/** Exposes the effects owner's explicit Target retry and cache adoption to callers. */
export function useRetryEffectTarget() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (targetId: string) =>
      retryEffectTarget(client, queryClient, targetId),
  });
}
