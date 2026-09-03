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
  const clearMutation = useMutation({
    mutationFn: () => client.proxy.clear({}),
    onSuccess: (response) => {
      queryClient.setQueryData(queryKeys.proxySettings, response);
    },
  });
  const checkMutation = useMutation({
    mutationFn: ({ url, settings }: { url: string; settings: ProxySettings }) =>
      client.proxy.check({ url, settings }),
  });

  const submit = useCallback(
    async (settings: ProxySettings) => {
      if (submissionPending.current) return;
      submissionPending.current = true;
      try {
        await mutation.mutateAsync(settings);
      } finally {
        submissionPending.current = false;
      }
    },
    [mutation],
  );

  const clear = useCallback(async () => {
    if (submissionPending.current) return;
    submissionPending.current = true;
    try {
      await clearMutation.mutateAsync();
    } finally {
      submissionPending.current = false;
    }
  }, [clearMutation]);

  const check = useCallback(
    (url: string, settings: ProxySettings) =>
      checkMutation.mutateAsync({ url, settings }),
    [checkMutation],
  );

  return {
    settings: query.data?.settings ?? null,
    isLoading: query.isPending,
    loadError: query.error,
    isSaving: mutation.isPending || clearMutation.isPending,
    isChecking: checkMutation.isPending,
    updateError: mutation.error ?? clearMutation.error,
    submit,
    clear,
    check,
    retry: query.refetch,
  };
}

export type ProxySettingsController = ReturnType<typeof useProxySettings>;
