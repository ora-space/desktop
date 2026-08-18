import { useEffect, useRef, useState } from "react";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  Toaster,
  TooltipProvider,
  type ResizablePanelHandle,
} from "@ora/ui";
import { QueryClientProvider } from "@tanstack/react-query";
import type { ContractsClient } from "@ora/contracts";
import type { ChatStore } from "@ora/chat";
import type { WorkflowRuntime } from "@ora/workflow-runtime";
import { PlatformProvider, type PlatformAdapter } from "./platform";
import { ContractsClientContext } from "./contracts-client-context";
import { ChatStoreContext } from "./chat-store-context";
import { WorkspaceSidebar } from "./features/workspace/workspace-sidebar";
import { WorkspaceView } from "./features/workspace/workspace-view";
import { WorkspaceDialogs } from "./features/workspace/workspace-dialogs";
import { SettingsDialog } from "./features/settings/settings-dialog";
import { SkillMarketplaceInstallController } from "./features/settings/skill-marketplace-install-controller";
import { TraceDashboardPanel } from "./features/trace-dashboard/trace-dashboard-panel";
import type {
  DashboardCompareResolver,
  DashboardResolver,
} from "./features/trace-dashboard/types";
import { AppI18nProvider } from "./i18n/i18n";
import type { CurrentUser } from "./lib/types";
import { createAppQueryClient } from "./state/query-client";
import { useGitIdentityUser } from "./state/hooks/use-git-identity";
import { useGraphWorkflowRunLiveSync } from "./state/hooks/use-graph-workflow-runs";
import { useSessionUnreadSync } from "./state/hooks/use-session-unread-sync";
import { useUiStore } from "./state/stores/ui-store";
import { startThemeSubscription } from "./state/stores/settings-store";
import { useTranslation } from "react-i18next";
import { WorkflowRuntimeProvider } from "./features/workflow-run/workflow-runtime-context";
import { AppEventGate } from "./state/app-event-gate";
export { AppEventGate } from "./state/app-event-gate";

interface AppShellProps {
  client: ContractsClient;
  chatStore: ChatStore;
  platform: PlatformAdapter;
  user?: CurrentUser;
  /** Runtime adapter; hosts will inject the generated-contract adapter once available. */
  workflowRuntime?: WorkflowRuntime;
  /** Desktop-injected resolver for the trace dashboard iframe URL; null when absent. */
  resolveDashboardUrl?: DashboardResolver | null;
  /** Desktop-injected resolver for the standalone token-comparison dashboard. */
  resolveDashboardCompareUrl?: DashboardCompareResolver | null;
}

const DEFAULT_SIDEBAR_WIDTH = 320;
const MIN_SIDEBAR_WIDTH = 240;
const MAX_SIDEBAR_WIDTH = 480;
const MIN_WORKSPACE_WIDTH = 480;

/** The main Ora application shell: sidebar + chat view with conversation state. */
export function AppShell({
  client,
  chatStore,
  platform,
  user,
  workflowRuntime,
  resolveDashboardUrl = null,
  resolveDashboardCompareUrl = null,
}: AppShellProps) {
  // One client per shell instance so HMR or multiple mounted shells never share cache.
  const [queryClient] = useState(() => createAppQueryClient());
  return (
    <QueryClientProvider client={queryClient}>
      <AppI18nProvider>
        <AppEventGate client={client}>
          <WorkflowRuntimeProvider runtime={workflowRuntime}>
            <AppShellContent
              client={client}
              chatStore={chatStore}
              platform={platform}
              user={user}
              resolveDashboardUrl={resolveDashboardUrl}
              resolveDashboardCompareUrl={resolveDashboardCompareUrl}
            />
          </WorkflowRuntimeProvider>
        </AppEventGate>
      </AppI18nProvider>
    </QueryClientProvider>
  );
}

/** Renders the shell inside providers so stateful hooks can consume the active locale. */
function AppShellContent({
  client,
  chatStore,
  platform,
  user: injectedUser,
  resolveDashboardUrl,
  resolveDashboardCompareUrl,
}: AppShellProps) {
  // Mirror theme/density onto <html> for the shell's lifetime.
  useEffect(() => startThemeSubscription(), []);
  // Track which sessions finished a turn while the user was looking elsewhere.
  useSessionUnreadSync(chatStore);
  // Keep sidebar / workspace run status current as the mock engine advances.
  useGraphWorkflowRunLiveSync();

  // Derive the sidebar user from the host's global Git identity unless a caller
  // (tests, storybook) injects an explicit user to render instead.
  const gitIdentityUser = useGitIdentityUser(
    client,
    injectedUser === undefined,
  );
  const user = injectedUser ?? gitIdentityUser;

  const sidebarCollapsed = useUiStore((s) => s.sidebarCollapsed);
  const { t } = useTranslation();
  const sidebarPanelRef = useRef<ResizablePanelHandle | null>(null);
  const handleSignOut = () => {
    chatStore.getState().clearAll();
    window.location.reload();
  };

  return (
    <ContractsClientContext.Provider value={client}>
      <ChatStoreContext.Provider value={chatStore}>
        <PlatformProvider adapter={platform}>
          <TooltipProvider>
            <a
              href="#main-content"
              className="fixed left-3 top-3 z-50 -translate-y-20 rounded-md bg-foreground px-3 py-2 text-sm text-background shadow-lg transition-transform focus:translate-y-0"
            >
              {t("common.skipToContent")}
            </a>
            <div className="flex h-dvh overflow-hidden bg-background text-foreground">
              {sidebarCollapsed ? (
                <WorkspaceView userName={user.name} />
              ) : (
                <ResizablePanelGroup orientation="horizontal">
                  <ResizablePanel
                    id="workspace-sidebar"
                    panelRef={sidebarPanelRef}
                    defaultSize={DEFAULT_SIDEBAR_WIDTH}
                    minSize={MIN_SIDEBAR_WIDTH}
                    maxSize={MAX_SIDEBAR_WIDTH}
                    groupResizeBehavior="preserve-pixel-size"
                  >
                    <WorkspaceSidebar user={user} onSignOut={handleSignOut} />
                  </ResizablePanel>
                  <ResizableHandle
                    withHandle
                    aria-label={t("sidebar.resize")}
                    title={t("sidebar.resize")}
                    className="z-20 bg-sidebar-border transition-colors hover:bg-ring focus-visible:bg-ring"
                    onDoubleClick={() =>
                      sidebarPanelRef.current?.resize(DEFAULT_SIDEBAR_WIDTH)
                    }
                  />
                  <ResizablePanel
                    id="workspace-content"
                    minSize={MIN_WORKSPACE_WIDTH}
                  >
                    <WorkspaceView userName={user.name} />
                  </ResizablePanel>
                </ResizablePanelGroup>
              )}
              <SettingsDialog />
              <SkillMarketplaceInstallController />
              <TraceDashboardPanel
                resolveDashboardUrl={resolveDashboardUrl ?? null}
                resolveDashboardCompareUrl={resolveDashboardCompareUrl ?? null}
              />
              {/* Mounted here, not in the sidebar, so collapsing the sidebar does
                  not take the workspace dialogs down with it. */}
              <WorkspaceDialogs />
            </div>
            <Toaster position="bottom-right" closeButton />
          </TooltipProvider>
        </PlatformProvider>
      </ChatStoreContext.Provider>
    </ContractsClientContext.Provider>
  );
}
