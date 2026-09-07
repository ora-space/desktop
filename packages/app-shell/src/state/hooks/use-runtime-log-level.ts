import { useCallback, useRef } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { RuntimeLogLevel } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { settingsKeys } from "../data/settings";

/** Loads and updates the process-wide runtime log level through the shared contracts client. */
export function useRuntimeLogLevel() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const submissionPending = useRef(false);
  const query = useQuery({
    queryKey: settingsKeys.runtimeLogLevel,
    queryFn: () => client.runtimeLogLevel.get({}),
  });
  const mutation = useMutation({
    mutationFn: (level: RuntimeLogLevel) =>
      client.runtimeLogLevel.set({ level }),
    onSuccess: (response) => {
      // Only a backend response may replace the authoritative cache entry.
      queryClient.setQueryData(settingsKeys.runtimeLogLevel, response);
    },
  });
  const submitLevel = useCallback(
    (level: RuntimeLogLevel) => {
      if (submissionPending.current) return;

      submissionPending.current = true;
      mutation.mutate(level, {
        onSettled: () => {
          submissionPending.current = false;
        },
      });
    },
    [mutation],
  );

  return {
    state: query.data,
    isLoading: query.isPending,
    loadError: query.error,
    isSaving: mutation.isPending,
    updateError: mutation.error,
    submitLevel,
  };
}
