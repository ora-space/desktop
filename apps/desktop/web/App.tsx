import { AppShell } from "@ora/app-shell";
import { createChatStore } from "@ora/chat";
import { createContractsClient } from "@ora/contracts";
import { createMockAcpClient, mockChatSuggestions, mockCurrentUser } from "@ora/mock-service";
import { createTauriPlatformAdapter } from "@ora/platform/tauri";
import { createTauriTransport } from "./tauri-transport";

const chatStore = createChatStore(createMockAcpClient());
const client = createContractsClient(createTauriTransport());
const platform = createTauriPlatformAdapter();

export default function App() {
  return <AppShell client={client} chatStore={chatStore} platform={platform} user={mockCurrentUser} chatSuggestions={mockChatSuggestions} />;
}
