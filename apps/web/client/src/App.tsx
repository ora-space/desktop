import { AppShell } from "@ora/app-shell";
import { chatStore, client, platform } from "@/contracts-runtime";

export default function App() {
  return <AppShell client={client} chatStore={chatStore} platform={platform} />;
}
