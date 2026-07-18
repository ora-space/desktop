import { AppShell } from "@ora/app-shell";
import { createContractsClient } from "@ora/contracts";
import { createMockTransport } from "@ora/mock-service";
import { createTauriPlatformAdapter } from "@ora/platform/tauri";

const client = createContractsClient(createMockTransport());
const platform = createTauriPlatformAdapter();

export default function App() {
  return <AppShell client={client} platform={platform} />;
}
