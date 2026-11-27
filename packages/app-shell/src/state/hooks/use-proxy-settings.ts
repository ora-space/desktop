import { useCallback, useRef } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type {
  GetProxySettingsResponse,
  ProxyMode,
  ProxySettings,
  SetProxySettingsRequest,
} from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { settingsKeys } from "../data/settings";

/** Loads and updates the host-level marketplace network proxy. */
export function useProxySettings() {
  const client = useContractsClient();
  const queryClient = useQueryClient();
  const submissionPending = useRef(false);
  const query = useQuery({
    queryKey: settingsKeys.proxySettings,
    queryFn: () => client.proxy.get({}),
  });
  const urlQuery = useQuery({
    queryKey: settingsKeys.proxyCheckUrl,
    queryFn: () => client.proxy.getCheckUrl({}),
  });
  const urlMutation = useMutation({
    mutationFn: (url: string) => client.proxy.setCheckUrl({ url }),
    onSuccess: async (response) => {
      await queryClient.cancelQueries({ queryKey: settingsKeys.proxyCheckUrl });
      queryClient.setQueryData(settingsKeys.proxyCheckUrl, response);
    },
  });
  const adoptSettings = async (response: GetProxySettingsResponse) => {
    // A pending read may have captured the settings before this write.
    await queryClient.cancelQueries({ queryKey: settingsKeys.proxySettings });
    queryClient.setQueryData(settingsKeys.proxySettings, response);
  };
  const mutation = useMutation({
    mutationFn: (request: SetProxySettingsRequest) => client.proxy.set(request),
    onSuccess: adoptSettings,
  });
  const clearMutation = useMutation({
    mutationFn: () => client.proxy.clear({}),
    onSuccess: adoptSettings,
  });
  const checkMutation = useMutation({
    mutationFn: ({
      url,
      settings,
      mode,
    }: {
      url: string;
      settings: ProxySettings | null;
      mode: ProxyMode;
    }) =>
      client.proxy.check({
        url,
        settings,
        ...(mode === "manual" ? {} : { mode }),
      }),
  });

  const submit = useCallback(
    async (mode: ProxyMode, settings: ProxySettings | null) => {
      if (submissionPending.current) return;
      submissionPending.current = true;
      try {
        await mutation.mutateAsync({
          settings,
          ...(mode === "manual" ? {} : { mode }),
        });
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
    (url: string, mode: ProxyMode, settings: ProxySettings | null) =>
      checkMutation.mutateAsync({ url, mode, settings }),
    [checkMutation],
  );

  return {
    settings: query.data?.settings ?? null,
    mode: query.data?.mode ?? (query.data?.settings ? "manual" : "system"),
    checkUrl: urlQuery.data?.url ?? "",
    hasLoaded: query.data !== undefined,
    isLoading: query.isPending,
    loadError: query.error,
    isCheckUrlLoading: urlQuery.isPending,
    checkUrlError: urlQuery.error,
    isSaving: mutation.isPending || clearMutation.isPending,
    isChecking: checkMutation.isPending || urlMutation.isPending,
    saveCheckUrl: urlMutation.mutateAsync,
    updateError: mutation.error ?? clearMutation.error,
    submit,
    clear,
    check,
    loadManual: async () =>
      (await client.proxy.get({ loadManual: true })).settings,
    retry: () => Promise.all([query.refetch(), urlQuery.refetch()]),
  };
}

export type ProxySettingsController = ReturnType<typeof useProxySettings>;
