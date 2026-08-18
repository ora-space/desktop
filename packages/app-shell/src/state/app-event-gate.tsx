import { type ReactNode } from "react";
import type { ContractsClient } from "@ora/contracts";
import { useTranslation } from "react-i18next";
import { useAppEvents } from "./hooks/use-app-events";

interface AppEventGateProps {
  client: ContractsClient;
  children: ReactNode;
}

/** Prevents normal application work until the Desktop event stream is ready. */
export function AppEventGate({ client, children }: AppEventGateProps) {
  return <AppEventStreamGate client={client}>{children}</AppEventStreamGate>;
}

/** Waits for the application event stream after this page owns the shell. */
function AppEventStreamGate({
  client,
  children,
}: {
  client: ContractsClient;
  children: ReactNode;
}) {
  const { ready } = useAppEvents(client);
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
