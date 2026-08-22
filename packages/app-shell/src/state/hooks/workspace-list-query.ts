/**
 * Shared `notifyOnChangeProps` for workspace list queries.
 *
 * Omits fetch-status flags so a background invalidate/refetch does not wake
 * every sidebar subscriber until `data` (or error/status) actually changes.
 */
export const WORKSPACE_LIST_NOTIFY_PROPS = [
  "data",
  "error",
  "isPending",
  "isSuccess",
  "status",
] as const;
