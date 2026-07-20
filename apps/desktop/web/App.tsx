import { AppShell } from "@ora/app-shell";
import { createChatStore } from "@ora/chat";
import { createContractsClient } from "@ora/contracts";
import { createMockAcpClient, createMockTransport } from "@ora/mock-service";
import { createTauriPlatformAdapter } from "@ora/platform/tauri";

const client = createContractsClient(createMockTransport());
const chatStore = createChatStore(createMockAcpClient());
const platform = createTauriPlatformAdapter();

export default function App() {
  return <AppShell client={client} chatStore={chatStore} platform={platform} />;
}
