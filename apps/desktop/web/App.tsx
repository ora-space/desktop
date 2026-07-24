import { AppShell } from "@ora/app-shell";
import { createChatStore } from "@ora/chat";
import { createContractsClient } from "@ora/contracts";
import { createTauriPlatformAdapter } from "@ora/platform/tauri";
import { invoke } from "@tauri-apps/api/core";
import type { DashboardEndpoint, DashboardResolver } from "@ora/app-shell";
import { createTauriTransport } from "./tauri-transport";

const client = createContractsClient(createTauriTransport());
const chatStore = createChatStore(client.session);
const platform = createTauriPlatformAdapter();

// Desktop-only resolver: asks the Ora backend to resolve the trace file, write the
// locator, probe the Streamlit server, and return the iframe URL. The agent session
// id stays in Rust; only the Ora session id + canonical agent type cross the wire.
const resolveDashboardUrl: DashboardResolver = async (sessionId: string) => {
  return invoke<DashboardEndpoint>("get_dashboard_url", {
    request: { sessionId },
  });
};

export default function App() {
  return (
    <AppShell
      client={client}
      chatStore={chatStore}
      platform={platform}
      resolveDashboardUrl={resolveDashboardUrl}
    />
  );
}
