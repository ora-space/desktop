import { useEffect } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type { CloudConnection } from "@ora/contracts";
import { useContractsClient } from "../../contracts-client-context";
import { useOptionalPlatform } from "../../platform";
import { settingsKeys } from "../data/settings";

/**
 * Loads the host-owned Cloud connection and exposes its use cases. Every host answer, and every
 * change the host announces (a sign-in finishing in the browser, a revoked token), replaces the
 * cached connection, so the panel never derives state the host did not report.
 */
export function useCloudConnection() {
  const client = useContractsClient();
  const platform = useOptionalPlatform();
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: settingsKeys.cloudConnection,
    queryFn: () => client.cloudConnection.get({}),
  });
  const adopt = (connection: CloudConnection) =>
    queryClient.setQueryData(settingsKeys.cloudConnection, connection);

  const cloudConnection = platform?.cloudConnection;
  useEffect(() => {
    if (cloudConnection === undefined) return;
    let disposed = false;
    let unsubscribe: (() => void) | undefined;
    void cloudConnection
      .onChanged((connection) => {
        if (!disposed)
          queryClient.setQueryData(settingsKeys.cloudConnection, connection);
      })
      .then((stop) => {
        if (disposed) stop();
        else unsubscribe = stop;
      })
      .catch(() => undefined);
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [cloudConnection, queryClient]);

  const setServer = useMutation({
    mutationFn: (server: string | null) =>
      client.cloudConnection.setServer({ server }),
    onSuccess: adopt,
  });
  const startLogin = useMutation({
    mutationFn: () => client.cloudConnection.startLogin({}),
    onSuccess: adopt,
  });
  const cancelLogin = useMutation({
    mutationFn: () => client.cloudConnection.cancelLogin({}),
    onSuccess: adopt,
  });
  const logout = useMutation({
    mutationFn: () => client.cloudConnection.logout({}),
    onSuccess: (response) => adopt(response.connection),
  });

  return {
    connection: query.data ?? null,
    isLoading: query.isPending,
    loadError: query.error,
    retry: query.refetch,
    saveServer: (server: string | null) => setServer.mutateAsync(server),
    saveError: setServer.error,
    isSavingServer: setServer.isPending,
    startLogin: () => startLogin.mutateAsync(),
    startError: startLogin.error,
    cancelLogin: () => cancelLogin.mutateAsync(),
    logout: () => logout.mutateAsync(),
    lastLogout: logout.data ?? null,
    isBusy:
      setServer.isPending ||
      startLogin.isPending ||
      cancelLogin.isPending ||
      logout.isPending,
  };
}

export type CloudConnectionController = ReturnType<typeof useCloudConnection>;
