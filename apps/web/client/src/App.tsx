import { AppShell } from "@ora/app-shell";
import { createContractsClient } from "@ora/contracts";
import { createMockTransport } from "@ora/mock-service";

const client = createContractsClient(createMockTransport());

export default function App() {
  return <AppShell client={client} />;
}
