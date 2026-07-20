import { useEffect, useRef, useState } from "react";
import {
  ResizableHandle,
  ResizablePanel,
  ResizablePanelGroup,
  TooltipProvider,
  type ResizablePanelHandle,
} from "@ora/ui";
import { QueryClientProvider } from "@tanstack/react-query";
import type { ContractsClient } from "@ora/contracts";
import type { ChatStore } from "@ora/chat";
import {
  PlatformHost,
  PlatformProvider,
  type PlatformAdapter,
  type PlatformLocale,
} from "@ora/platform";
import { ContractsClientContext } from "./contracts-client-context";
import { ChatStoreContext } from "./chat-store-context";
import { WorkspaceSidebar } from "./features/workspace/workspace-sidebar";
import { WorkspaceView } from "./features/workspace/workspace-view";
import { SettingsDialog } from "./features/settings/settings-dialog";
import { AppI18nProvider } from "./i18n/i18n";
import { CURRENT_USER } from "./lib/mock-data";
import type { CurrentUser } from "./lib/types";
import { createAppQueryClient } from "./state/query-client";
import { useUiStore } from "./state/stores/ui-store";
import { startThemeSubscription } from "./state/stores/settings-store";
import { useTranslation } from "react-i18next";

interface AppShellProps {
  client: ContractsClient;
  chatStore: ChatStore;
  platform: PlatformAdapter;
  user?: CurrentUser;
}

const DEFAULT_SIDEBAR_WIDTH = 320;
const MIN_SIDEBAR_WIDTH = 240;
const MAX_SIDEBAR_WIDTH = 480;
const MIN_WORKSPACE_WIDTH = 480;

/** The main Ora application shell: sidebar + chat view with conversation state. */
export function AppShell({ client, chatStore, platform, user = CURRENT_USER }: AppShellProps) {
  // One client per shell instance so HMR or multiple mounted shells never share cache.
  const [queryClient] = useState(() => createAppQueryClient());
  return (
    <QueryClientProvider client={queryClient}>
      <AppI18nProvider>
        <AppShellContent client={client} chatStore={chatStore} platform={platform} user={user} />
      </AppI18nProvider>
    </QueryClientProvider>
  );
}

/** Renders the shell inside providers so stateful hooks can consume the active locale. */
function AppShellContent({ client, chatStore, platform, user }: Required<AppShellProps>) {
  // Mirror theme/density onto <html> for the shell's lifetime.
  useEffect(() => startThemeSubscription(), []);

  const sidebarCollapsed = useUiStore((s) => s.sidebarCollapsed);
  const { i18n, t } = useTranslation();
  const sidebarPanelRef = useRef<ResizablePanelHandle | null>(null);
  const locale: PlatformLocale = i18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN";

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
                    onDoubleClick={() => sidebarPanelRef.current?.resize(DEFAULT_SIDEBAR_WIDTH)}
                  />
                  <ResizablePanel id="workspace-content" minSize={MIN_WORKSPACE_WIDTH}>
                    <WorkspaceView userName={user.name} />
                  </ResizablePanel>
                </ResizablePanelGroup>
              )}
              <SettingsDialog />
            </div>
            <PlatformHost locale={locale} />
          </TooltipProvider>
        </PlatformProvider>
      </ChatStoreContext.Provider>
    </ContractsClientContext.Provider>
  );
}
