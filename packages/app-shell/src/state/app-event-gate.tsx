import { type ReactNode } from "react";
import type { ChatStore } from "@ora/chat";
import type { ContractsClient } from "@ora/contracts";
import { useTranslation } from "react-i18next";
import { useAppEvents } from "./hooks/use-app-events";

interface AppEventGateProps {
  client: ContractsClient;
  /** Told about sessions whose options may have changed. */
  chatStore?: ChatStore;
  children: ReactNode;
}

/** Prevents normal application work until the Desktop event stream is ready. */
export function AppEventGate({ client, chatStore, children }: AppEventGateProps) {
  return (
    <AppEventStreamGate client={client} chatStore={chatStore}>
      {children}
    </AppEventStreamGate>
  );
}

/** Waits for the application event stream after this page owns the shell. */
function AppEventStreamGate({
  client,
  chatStore,
  children,
}: {
  client: ContractsClient;
  chatStore?: ChatStore;
  children: ReactNode;
}) {
  const { ready } = useAppEvents(client, chatStore);
  if (!ready) return <Connecting />;
  return <>{children}</>;
}

/** Renders the shared startup state used while application events are connecting. */
function Connecting() {
  const { t } = useTranslation();
  return (
    <main className="flex min-h-dvh items-center justify-center bg-background text-sm text-muted-foreground">
      {t("appEvents.connecting")}
    </main>
  );
}
