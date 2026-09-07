import { useCallback, useRef } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useContractsClient } from "../../contracts-client-context";
import { settingsKeys } from "../data/settings";

/** Loads and updates the shared developer-mode preference through the contracts client. */
export function useDeveloperMode() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const submissionPending = useRef(false);
  const query = useQuery({
    queryKey: settingsKeys.developerMode,
    queryFn: () => client.developerMode.get({}),
  });
  const mutation = useMutation({
    mutationFn: (enabled: boolean) => client.developerMode.set({ enabled }),
    onSuccess: (response) => {
      // Keep the server response authoritative; optimistic state could expose controls
      // even when persistence rejected the requested change.
      queryClient.setQueryData(settingsKeys.developerMode, response);
    },
  });
  const submitEnabled = useCallback(
    (enabled: boolean) => {
      if (submissionPending.current) return;

      submissionPending.current = true;
      mutation.mutate(enabled, {
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
    submitEnabled,
    retry: query.refetch,
  };
}

export type DeveloperModeController = ReturnType<typeof useDeveloperMode>;
