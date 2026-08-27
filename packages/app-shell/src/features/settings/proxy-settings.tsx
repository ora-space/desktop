import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Button, Input } from "@ora/ui";
import { IconServer } from "@tabler/icons-react";
import type { ProxySettings } from "@ora/contracts";
import {
  type ProxySettingsController,
  useProxySettings,
} from "../../state/hooks/use-proxy-settings";
import { SettingsHeading } from "./settings-heading";

interface ProxySettingsEditorProps {
  initialSettings: ProxySettings | null;
  controller: ProxySettingsController;
}

/** Present a host-level proxy editor whose saved value only marketplace sources may opt into. */
export function ProxySettings() {
  const { t } = useTranslation();
  const controller = useProxySettings();

  const settingsKey = controller.settings
    ? [
        controller.settings.host,
        String(controller.settings.port),
        controller.settings.username ?? "",
        controller.settings.password ?? "",
      ].join("\u0000")
    : "empty";

  return (
    <div className="space-y-6">
      <SettingsHeading
        title={t("settings.proxy.title")}
        description={t("settings.proxy.description")}
      />

      {controller.isLoading ? (
        <span role="status" className="text-xs text-muted-foreground">
          {t("settings.proxy.loading")}
        </span>
      ) : (
        <ProxySettingsEditor
          key={settingsKey}
          initialSettings={controller.settings}
          controller={controller}
        />
      )}
    </div>
  );
}

function ProxySettingsEditor({
  initialSettings,
  controller,
}: ProxySettingsEditorProps) {
  const { t } = useTranslation();
  const [host, setHost] = useState(initialSettings?.host ?? "");
  const [port, setPort] = useState(
    initialSettings ? String(initialSettings.port) : "",
  );
  const [username, setUsername] = useState(initialSettings?.username ?? "");
  const [password, setPassword] = useState(initialSettings?.password ?? "");

  const save = () => {
    const parsedPort = Number(port);
    if (
      host.trim() === "" ||
      !Number.isInteger(parsedPort) ||
      parsedPort <= 0 ||
      parsedPort > 65535
    ) {
      return;
    }
    controller.submit({
      host: host.trim(),
      port: parsedPort,
      username: username.trim() === "" ? null : username.trim(),
      password: password === "" ? null : password,
    });
  };

  return (
    <section className="rounded-lg border border-border/70 bg-muted/25 p-4">
      <div className="flex items-start gap-3">
        <IconServer className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1 space-y-4">
          <div className="grid gap-3 sm:grid-cols-[1fr_120px]">
            <label className="space-y-1.5">
              <span className="text-sm font-medium">
                {t("settings.proxy.host")}
              </span>
              <Input
                value={host}
                onChange={(event) => setHost(event.target.value)}
                placeholder={t("settings.proxy.hostPlaceholder")}
                aria-label={t("settings.proxy.host")}
                autoComplete="off"
              />
            </label>
            <label className="space-y-1.5">
              <span className="text-sm font-medium">
                {t("settings.proxy.port")}
              </span>
              <Input
                value={port}
                onChange={(event) => setPort(event.target.value)}
                placeholder={t("settings.proxy.portPlaceholder")}
                aria-label={t("settings.proxy.port")}
                inputMode="numeric"
              />
            </label>
          </div>
          <div className="grid gap-3 sm:grid-cols-2">
            <label className="space-y-1.5">
              <span className="text-sm font-medium">
                {t("settings.proxy.username")}
              </span>
              <Input
                value={username}
                onChange={(event) => setUsername(event.target.value)}
                placeholder={t("settings.proxy.optional")}
                aria-label={t("settings.proxy.username")}
                autoComplete="off"
              />
            </label>
            <label className="space-y-1.5">
              <span className="text-sm font-medium">
                {t("settings.proxy.password")}
              </span>
              <Input
                type="password"
                value={password}
                onChange={(event) => setPassword(event.target.value)}
                placeholder={t("settings.proxy.optional")}
                aria-label={t("settings.proxy.password")}
                autoComplete="new-password"
              />
            </label>
          </div>
          <div className="flex flex-wrap items-center gap-3">
            <Button
              variant="outline"
              disabled={
                controller.isLoading ||
                controller.isSaving ||
                host.trim() === "" ||
                port.trim() === ""
              }
              onClick={save}
            >
              {controller.isSaving
                ? t("settings.proxy.saving")
                : t("settings.proxy.save")}
            </Button>
            {controller.loadError !== null && (
              <span role="alert" className="text-xs text-destructive">
                {t("settings.proxy.loadError")}
              </span>
            )}
            {controller.updateError !== null && (
              <span role="alert" className="text-xs text-destructive">
                {t("settings.proxy.updateError")}
              </span>
            )}
          </div>
        </div>
      </div>
    </section>
  );
}
