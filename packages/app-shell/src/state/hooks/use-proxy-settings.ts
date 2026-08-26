import { useCallback, useRef } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { ProxySettings } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { queryKeys } from "./query-keys";

/** Loads and updates the host-level marketplace network proxy. */
export function useProxySettings() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const submissionPending = useRef(false);
  const query = useQuery({
    queryKey: queryKeys.proxySettings,
    queryFn: () => client.proxy.get({}),
  });
  const mutation = useMutation({
    mutationFn: (settings: ProxySettings) => client.proxy.set({ settings }),
    onSuccess: (response) => {
      // The backend response is authoritative, including a null settings value.
      queryClient.setQueryData(queryKeys.proxySettings, response);
    },
  });

  const submit = useCallback(
    (settings: ProxySettings) => {
      if (submissionPending.current) return;
      submissionPending.current = true;
      mutation.mutate(settings, {
        onSettled: () => {
          submissionPending.current = false;
        },
      });
    },
    [mutation],
  );

  return {
    settings: query.data?.settings ?? null,
    isLoading: query.isPending,
    loadError: query.error,
    isSaving: mutation.isPending,
    updateError: mutation.error,
    submit,
    retry: query.refetch,
  };
}

export type ProxySettingsController = ReturnType<typeof useProxySettings>;
