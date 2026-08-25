import { useQuery } from "@tanstack/react-query";
import { useOptionalPlatform } from "../../platform";
import { queryKeys } from "./query-keys";

/** Resolves a Workspace's local execution directory through the injected host adapter. */
export function useWorkspaceCwd(workspaceId: string | undefined) {
  const platform = useOptionalPlatform();
  return useQuery({
    queryKey: queryKeys.workspaceCwd(workspaceId ?? ""),
    queryFn: () => {
      if (platform === null || workspaceId === undefined) {
        return Promise.reject(
          new Error("Workspace path resolver not available"),
        );
      }
      return platform.locationActions.resolveWorkspaceCwd(workspaceId);
    },
    enabled: workspaceId !== undefined && platform !== null,
    retry: false,
  });
}
