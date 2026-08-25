export { AppShell } from "./app-shell";
export { PlatformProvider, usePlatform } from "./platform";
export {
  PathSelectionInProgressError,
  type LocationActionsCapability,
  type LocationTarget,
  type PathSelectionKind,
  type PlatformAdapter,
  type SaveTextFileOptions,
  type SelectPathOptions,
  type SurfaceBounds,
  type SurfaceCapability,
  type SurfaceOpenTarget,
  type SurfaceKind,
  type DownloadAction,
  type ResolveDownloadOutcome,
  type SurfaceEvent,
  type SurfaceRecord,
  type SurfaceState,
  type SurfaceTarget,
  type WindowControlsCapability,
  type WindowManagerOs,
  type WorktreeStorageCapability,
} from "./platform";
export type {
  DashboardCompareResolver,
  DashboardEndpoint,
  DashboardResolver,
} from "./features/trace-dashboard/types";
export { AppI18nProvider, type Locale, type TranslationKey } from "./i18n/i18n";
export { appI18n } from "./i18n/i18n-instance";
export { useContractsClient } from "./contracts-client-context";
export { useChatStore } from "./chat-store-context";
export { createAppQueryClient } from "./state/query-client";
