import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Input } from "@ora/ui";
import type { CloudConnection, CloudSession } from "@ora/contracts";
import { localizeContractError } from "../../i18n/contract-error";
import {
  type CloudConnectionController,
  useCloudConnection,
} from "../../state/hooks/use-cloud-connection";
import { SettingsHeading } from "./settings-heading";

/** Server address and sign-in for the Cloud connection; local work never depends on it. */
export function CloudConnectionSettings() {
  const { t } = useTranslation();
  const controller = useCloudConnection();
  const { connection } = controller;
  const server = connection?.kind === "configured" ? connection.server : "";

  return (
    <div className="space-y-6">
      <SettingsHeading
        title={t("settings.cloud.title")}
        description={t("settings.cloud.description")}
      />
      {controller.isLoading ? (
        <span role="status" className="text-xs text-muted-foreground">
          {t("settings.cloud.loading")}
        </span>
      ) : connection === null ? (
        <div className="flex items-center gap-2">
          <p role="alert" className="text-xs text-destructive">
            {t("settings.cloud.loadError")}
          </p>
          <Button
            size="sm"
            variant="outline"
            onClick={() => void controller.retry()}
          >
            {t("common.retry")}
          </Button>
        </div>
      ) : (
        <>
          <ServerEditor
            key={server}
            initialServer={server}
            controller={controller}
          />
          <SessionRow connection={connection} controller={controller} />
        </>
      )}
    </div>
  );
}

/** Edits the saved address; saving a different one signs out of the previous server. */
function ServerEditor({
  initialServer,
  controller,
}: {
  initialServer: string;
  controller: CloudConnectionController;
}) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState(initialServer);
  const trimmed = draft.trim();
  const unchanged = trimmed === initialServer;

  return (
    <section className="space-y-2">
      <label htmlFor="cloud-server" className="text-sm font-medium">
        {t("settings.cloud.server")}
      </label>
      <div className="flex gap-2">
        <Input
          id="cloud-server"
          value={draft}
          placeholder="https://cloud.example.com"
          onChange={(event) => setDraft(event.target.value)}
        />
        <Button
          disabled={unchanged || controller.isBusy}
          onClick={() =>
            void controller
              .saveServer(trimmed === "" ? null : trimmed)
              .catch(() => undefined)
          }
        >
          {t("settings.cloud.save")}
        </Button>
      </div>
      <p className="text-xs leading-5 text-muted-foreground">
        {t("settings.cloud.serverHint")}
      </p>
      {controller.saveError !== null && (
        <p role="alert" className="text-xs text-destructive">
          {t("settings.cloud.saveError", {
            reason: localizeContractError(controller.saveError, t),
          })}
        </p>
      )}
    </section>
  );
}

/** Shows the sign-in state and the one action that state allows. */
function SessionRow({
  connection,
  controller,
}: {
  connection: CloudConnection;
  controller: CloudConnectionController;
}) {
  const { t } = useTranslation();
  if (connection.kind === "notConfigured") {
    return (
      <p role="status" className="text-sm text-muted-foreground">
        {t("settings.cloud.status.notConfigured")}
      </p>
    );
  }
  const session = connection.session;
  const action = sessionAction(session);
  const revocationUnconfirmed =
    session.kind === "signedOut" &&
    controller.lastLogout?.revocation === "unconfirmed";

  return (
    <section className="space-y-2 border-y border-border py-4">
      <div className="flex items-center gap-4">
        <p role="status" className="min-w-0 flex-1 text-sm">
          {sessionText(session, t)}
        </p>
        {action === "signIn" && (
          <Button
            disabled={controller.isBusy}
            onClick={() => void controller.startLogin().catch(() => undefined)}
          >
            {t("settings.cloud.signIn")}
          </Button>
        )}
        {action === "cancel" && (
          <Button
            variant="outline"
            disabled={controller.isBusy}
            onClick={() => void controller.cancelLogin()}
          >
            {t("settings.cloud.cancel")}
          </Button>
        )}
        {action === "signOut" && (
          <Button
            variant="outline"
            disabled={controller.isBusy}
            onClick={() => void controller.logout()}
          >
            {t("settings.cloud.signOut")}
          </Button>
        )}
      </div>
      {revocationUnconfirmed && (
        <p className="text-xs text-muted-foreground">
          {t("settings.cloud.revocationUnconfirmed")}
        </p>
      )}
      {controller.startError !== null && session.kind !== "signingIn" && (
        <p role="alert" className="text-xs text-destructive">
          {t("settings.cloud.startError")}
        </p>
      )}
    </section>
  );
}

type SessionAction = "signIn" | "cancel" | "signOut";

/** Which control a session offers; the host refuses anything else anyway. */
function sessionAction(session: CloudSession): SessionAction {
  switch (session.kind) {
    case "signedOut":
    case "needsSignIn":
    case "storageUnavailable":
      return "signIn";
    case "signingIn":
      return "cancel";
    case "signedIn":
    case "unreachable":
      return "signOut";
  }
}

/** The status sentence for each session the host can report. */
function sessionText(
  session: CloudSession,
  t: ReturnType<typeof useTranslation>["t"],
): string {
  switch (session.kind) {
    case "signedOut":
      return session.failure === null
        ? t("settings.cloud.status.signedOut")
        : t(`settings.cloud.failure.${session.failure}`);
    case "signingIn":
      return t("settings.cloud.status.signingIn");
    case "signedIn":
      return t("settings.cloud.status.signedIn", {
        name: session.account.displayName,
      });
    case "unreachable":
      return t("settings.cloud.status.unreachable", {
        name: session.account.displayName,
      });
    case "needsSignIn":
      return t("settings.cloud.status.needsSignIn");
    case "storageUnavailable":
      return t("settings.cloud.status.storageUnavailable", {
        reason: session.reason,
      });
  }
}
