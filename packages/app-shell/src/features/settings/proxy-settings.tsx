import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
  Input,
  toast,
} from "@ora/ui";
import { IconServer } from "@tabler/icons-react";
import type { CheckProxySettingsResponse, ProxySettings } from "@ora/contracts";
import { localizeContractError } from "../../i18n/contract-error";
import {
  type ProxySettingsController,
  useProxySettings,
} from "../../state/hooks/use-proxy-settings";
import { SettingsHeading } from "./settings-heading";

const DEFAULT_CHECK_URL = "https://example.com/";

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

/** Reads the current form into a validated proxy value, or `null` when host/port are unusable. */
function parsedProxySettings(
  host: string,
  port: string,
  username: string,
  password: string,
): ProxySettings | null {
  const parsedPort = Number(port);
  if (
    host.trim() === "" ||
    !Number.isInteger(parsedPort) ||
    parsedPort <= 0 ||
    parsedPort > 65535
  ) {
    return null;
  }
  return {
    host: host.trim(),
    port: parsedPort,
    username: username.trim() === "" ? null : username.trim(),
    password: password === "" ? null : password,
  };
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
  const [checkOpen, setCheckOpen] = useState(false);

  const parsed = parsedProxySettings(host, port, username, password);
  const hasFormValues =
    host.trim() !== "" ||
    port.trim() !== "" ||
    username.trim() !== "" ||
    password !== "";
  const busy = controller.isLoading || controller.isSaving;

  const save = async () => {
    if (parsed === null) {
      toast.error(t("settings.proxy.invalid"));
      return;
    }
    try {
      await controller.submit(parsed);
      toast.success(t("settings.proxy.saved"));
    } catch (cause) {
      toast.error(t("settings.proxy.updateError"), {
        description: localizeContractError(cause, t),
      });
    }
  };

  const clear = async () => {
    try {
      await controller.clear();
      toast.success(t("settings.proxy.cleared"));
    } catch (cause) {
      toast.error(t("settings.proxy.clearError"), {
        description: localizeContractError(cause, t),
      });
    }
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
              disabled={busy || parsed === null}
              onClick={() => void save()}
            >
              {controller.isSaving
                ? t("settings.proxy.saving")
                : t("settings.proxy.save")}
            </Button>
            <Button
              variant="outline"
              disabled={
                busy || (!hasFormValues && controller.settings === null)
              }
              onClick={() => void clear()}
            >
              {t("settings.proxy.clear")}
            </Button>
            <Button
              variant="outline"
              disabled={busy || parsed === null}
              onClick={() => setCheckOpen(true)}
            >
              {t("settings.proxy.check")}
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
      {checkOpen && parsed !== null && (
        <CheckProxyDialog
          checking={controller.isChecking}
          onOpenChange={setCheckOpen}
          onCheck={(url) => controller.check(url, parsed)}
        />
      )}
    </section>
  );
}

interface CheckProxyDialogProps {
  checking: boolean;
  onOpenChange: (open: boolean) => void;
  onCheck: (url: string) => Promise<CheckProxySettingsResponse>;
}

/** Prompts for a URL and reports whether the current form proxy can reach it. */
function CheckProxyDialog({
  checking,
  onOpenChange,
  onCheck,
}: CheckProxyDialogProps) {
  const { t } = useTranslation();
  const [url, setUrl] = useState(DEFAULT_CHECK_URL);

  const verify = async () => {
    const nextUrl = url.trim();
    if (nextUrl === "") {
      toast.error(t("settings.proxy.checkUrlRequired"));
      return;
    }
    try {
      const result = await onCheck(nextUrl);
      if (result.outcome === "reachable") {
        toast.success(t("settings.proxy.checkSuccess"), {
          description: t("settings.proxy.checkSuccessDetail", {
            status: result.status,
          }),
        });
        onOpenChange(false);
        return;
      }
      toast.error(t("settings.proxy.checkFailed"), {
        description: result.message,
      });
    } catch (cause) {
      toast.error(t("settings.proxy.checkFailed"), {
        description: localizeContractError(cause, t),
      });
    }
  };

  return (
    <Dialog open onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>{t("settings.proxy.checkTitle")}</DialogTitle>
          <DialogDescription>
            {t("settings.proxy.checkDescription")}
          </DialogDescription>
        </DialogHeader>
        <label className="space-y-1.5">
          <span className="text-sm font-medium">
            {t("settings.proxy.checkUrl")}
          </span>
          <Input
            value={url}
            onChange={(event) => setUrl(event.target.value)}
            aria-label={t("settings.proxy.checkUrl")}
            autoComplete="off"
            onKeyDown={(event) => {
              if (event.key === "Enter") {
                event.preventDefault();
                void verify();
              }
            }}
          />
        </label>
        <DialogFooter>
          <Button
            variant="outline"
            disabled={checking}
            onClick={() => onOpenChange(false)}
          >
            {t("common.cancel")}
          </Button>
          <Button
            disabled={checking || url.trim() === ""}
            onClick={() => void verify()}
          >
            {checking
              ? t("settings.proxy.checking")
              : t("settings.proxy.checkConfirm")}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
