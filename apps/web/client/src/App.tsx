import { AppShell } from "@ora/app-shell";
import { client, platform } from "@/contracts-runtime";

export default function App() {
  return <AppShell client={client} platform={platform} />;
}
