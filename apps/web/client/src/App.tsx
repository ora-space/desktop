import { AppShell } from "@ora/app-shell";
import { chatStore, chatSuggestions, client, currentUser, platform } from "@/contracts-runtime";

export default function App() {
  return <AppShell client={client} chatStore={chatStore} platform={platform} user={currentUser} chatSuggestions={chatSuggestions} />;
}
