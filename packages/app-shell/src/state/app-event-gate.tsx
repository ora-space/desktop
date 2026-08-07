import type { ReactNode } from "react";
import type { ContractsClient } from "@ora/contracts";
import { Button } from "@ora/ui";
import { useTranslation } from "react-i18next";
import { useAppEvents } from "./hooks/use-app-events";

/** Prevents normal application work until this document owns the app event stream. */
export function AppEventGate({ client, children }: { client: ContractsClient; children: ReactNode }) {
  const { ready, multipleClientsUnsupported, retry } = useAppEvents(client);
  const { t } = useTranslation();

  if (multipleClientsUnsupported) {
    return (
      <main className="flex min-h-dvh items-center justify-center bg-background px-6 text-foreground">
        <section className="w-full max-w-md space-y-4 rounded-xl border border-border bg-card p-6 shadow-sm">
          <h1 className="text-xl font-semibold">{t("appEvents.multipleClients.title")}</h1>
          <p className="text-sm text-muted-foreground">{t("appEvents.multipleClients.description")}</p>
          <Button type="button" onClick={retry}>{t("appEvents.multipleClients.retry")}</Button>
        </section>
      </main>
    );
  }

  if (!ready) {
    return (
      <main className="flex min-h-dvh items-center justify-center bg-background text-sm text-muted-foreground">
        {t("appEvents.connecting")}
      </main>
    );
  }

  return <>{children}</>;
}
