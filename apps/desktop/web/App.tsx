import { AppShell } from "@ora/app-shell";
import { createChatStore } from "@ora/chat";
import { createContractsClient } from "@ora/contracts";
import { createTauriTransport } from "./tauri-transport";
import { createTauriPlatformAdapter } from "./tauri-platform-adapter";

const client = createContractsClient(createTauriTransport());
const chatStore = createChatStore(client.session);
const platform = createTauriPlatformAdapter();

export default function App() {
  return <AppShell client={client} chatStore={chatStore} platform={platform} />;
}
