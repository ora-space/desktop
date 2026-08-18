import { AppShell } from "@ora/app-shell";
import { createChatStore } from "@ora/chat";
import { createContractsClient } from "@ora/contracts";
import { invoke } from "@tauri-apps/api/core";
import type {
  DashboardCompareResolver,
  DashboardEndpoint,
  DashboardResolver,
} from "@ora/app-shell";
import { createTauriTransport } from "./tauri-transport";
import { createTauriPlatformAdapter } from "./tauri-platform-adapter";

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

const resolveDashboardCompareUrl: DashboardCompareResolver = async () => {
  return invoke<DashboardEndpoint>("get_dashboard_compare_url");
};

export default function App() {
  return (
    <AppShell
      client={client}
      chatStore={chatStore}
      platform={platform}
      resolveDashboardUrl={resolveDashboardUrl}
      resolveDashboardCompareUrl={resolveDashboardCompareUrl}
    />
  );
}
