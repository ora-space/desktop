import { useId, useState } from "react";
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
import type {
  CheckProxySettingsResponse,
  ProxyMode,
  ProxySettings,
} from "@ora/contracts";
import { localizeContractError } from "../../i18n/contract-error";
import {
  type ProxySettingsController,
  useProxySettings,
} from "../../state/hooks/use-proxy-settings";
import { SettingsHeading } from "./settings-heading";

interface ProxySettingsEditorProps {
  initialSettings: ProxySettings | null;
  controller: ProxySettingsController;
  replacingUnreadable: boolean;
  onReplaceUnreadable: () => void;
}

/** Edits the host proxy used by Desktop updates and opted-in marketplace sources. */
export function ProxySettings() {
  const { t } = useTranslation();
  const controller = useProxySettings();
  const [replacementError, setReplacementError] = useState<Error | null>(null);
  const replacingUnreadable =
    controller.loadError !== null && controller.loadError === replacementError;
  const replaceUnreadable = () => setReplacementError(controller.loadError);

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

      {!controller.hasLoaded && controller.isLoading ? (
        <span role="status" className="text-xs text-muted-foreground">
          {t("settings.proxy.loading")}
        </span>
      ) : !controller.hasLoaded &&
        controller.loadError !== null &&
        !replacingUnreadable ? (
        <div className="space-y-3">
          <p role="alert" className="text-sm text-destructive">
            {t("settings.proxy.loadError")}
          </p>
          <Button variant="outline" onClick={() => void controller.retry()}>
            {t("common.retry")}
          </Button>
          <Button variant="outline" onClick={replaceUnreadable}>
            {t("settings.proxy.replaceUnreadable")}
          </Button>
        </div>
      ) : (
        <ProxySettingsEditor
          key={controller.mode + settingsKey}
          initialSettings={controller.settings}
          controller={controller}
          replacingUnreadable={replacingUnreadable}
          onReplaceUnreadable={replaceUnreadable}
        />
      )}
    </div>
  );
}

/** Splits pasted addresses into form fields, keeping schemes that affect routing. */
function splitProxyAddress(input: string) {
  const address = input.trim();
  const parts = /^(?:(?:https?|socks5h?):\/\/)?([^/?#\\\s]+)\/?$/i.exec(
    address,
  );
  if (parts === null) return null;
  const [, authority] = parts;
  try {
    const url = new URL(
      address.includes("://") ? address : "http://" + address,
    );
    const hostAndPort = authority.slice(authority.lastIndexOf("@") + 1);
    // URL.port omits explicit defaults such as :80 and :443.
    const rawPort = /:(\d+)$/.exec(hostAndPort)?.[1];
    if (url.hostname === "") return null;
    const username = decodeURIComponent(url.username);
    const password = decodeURIComponent(url.password);
    // Single-line inputs cannot display line breaks from encoded URL credentials.
    if (/[\r\n]/.test(username) || /[\r\n]/.test(password)) return null;
    return {
      host:
        url.protocol === "http:"
          ? url.hostname
          : url.protocol + "//" + url.hostname,
      port: rawPort === undefined ? "" : String(Number(rawPort)),
      credentials: authority.includes("@") ? { username, password } : null,
    };
  } catch {
    return null;
  }
}

/** Accepts the decimal TCP port expected by the proxy settings contract. */
function parseProxyPort(input: string): number | null {
  const value = input.trim();
  const port = Number(value);
  return /^\d+$/.test(value) && port >= 1 && port <= 65535 ? port : null;
}

function ProxySettingsEditor({
  initialSettings,
  controller,
  replacingUnreadable,
  onReplaceUnreadable,
}: ProxySettingsEditorProps) {
  const { t } = useTranslation();
  const [mode, setMode] = useState<ProxyMode>(controller.mode);
  const [host, setHost] = useState(initialSettings?.host ?? "");
  const [port, setPort] = useState(
    initialSettings ? String(initialSettings.port) : "",
  );
  const [username, setUsername] = useState(initialSettings?.username ?? "");
  const [password, setPassword] = useState(initialSettings?.password ?? "");
  const [manualLoaded, setManualLoaded] = useState(initialSettings !== null);
  const [manualLoading, setManualLoading] = useState(false);
  const [manualLoadError, setManualLoadError] = useState(false);
  const [checkOpen, setCheckOpen] = useState(false);
  const [checkUrl, setCheckUrl] = useState(controller.checkUrl);
  const [unusedAddressPort, setUnusedAddressPort] = useState<string | null>(
    null,
  );
  const addressHintId = useId();
  const portHintId = useId();

  const address = mode === "manual" ? splitProxyAddress(host) : null;
  const portEntered = port.trim() !== "";
  const fieldPort = parseProxyPort(port);
  const parsedPort = portEntered
    ? fieldPort
    : parseProxyPort(address?.port ?? "");
  const credentials = address?.credentials ?? { username, password };
  const parsed: ProxySettings | null =
    address === null || parsedPort === null
      ? null
      : {
          host: address.host,
          port: parsedPort,
          username: credentials.username.trim() || null,
          password: credentials.password || null,
        };
  const invalidAddress =
    mode === "manual" && host.trim() !== "" && address === null;
  const invalidPort = portEntered && fieldPort === null;
  const invalid = mode === "manual" && parsed === null;
  const usingDraft =
    mode !== controller.mode ||
    (parsed !== null &&
      (initialSettings === null ||
        parsed.host !== splitProxyAddress(initialSettings.host)?.host ||
        parsed.port !== initialSettings.port ||
        parsed.username !== initialSettings.username ||
        parsed.password !== initialSettings.password));
  const busy =
    controller.isLoading ||
    controller.isSaving ||
    manualLoading ||
    (controller.loadError !== null && !replacingUnreadable);
  const checkBlocked =
    busy ||
    replacingUnreadable ||
    controller.isCheckUrlLoading ||
    controller.checkUrlError !== null;

  const applyAddress = (value: NonNullable<typeof address>) => {
    setHost(value.host);
    if (!portEntered) {
      setPort(value.port);
    } else if (value.port !== "" && fieldPort !== Number(value.port)) {
      setUnusedAddressPort(value.port);
    }
    if (value.credentials !== null) {
      setUsername(value.credentials.username);
      setPassword(value.credentials.password);
    }
  };

  const selectMode = async (next: ProxyMode) => {
    setMode(next);
    if (next !== "manual" || manualLoaded) return;
    setManualLoading(true);
    try {
      const saved = await controller.loadManual();
      setHost(saved?.host ?? "");
      setPort(saved ? String(saved.port) : "");
      setUsername(saved?.username ?? "");
      setPassword(saved?.password ?? "");
    } catch {
      setManualLoadError(true);
    } finally {
      setManualLoaded(true);
      setManualLoading(false);
    }
  };

  const save = async () => {
    if (invalid) {
      toast.error(t("settings.proxy.invalid"));
      return;
    }
    try {
      await controller.submit(mode, parsed);
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
      setMode("system");
      setHost("");
      setPort("");
      setUnusedAddressPort(null);
      setUsername("");
      setPassword("");
      setManualLoadError(false);
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
          <label className="block space-y-1.5">
            <span className="text-sm font-medium">
              {t("settings.proxy.mode")}
            </span>
            <select
              className="block w-full rounded-md border bg-background px-3 py-2 text-sm"
              value={mode}
              disabled={busy}
              onChange={(event) => {
                const value = event.currentTarget.value;
                if (
                  value === "system" ||
                  value === "manual" ||
                  value === "direct"
                )
                  void selectMode(value);
              }}
            >
              <option value="system">{t("settings.proxy.modeSystem")}</option>
              <option value="manual">{t("settings.proxy.modeManual")}</option>
              <option value="direct">{t("settings.proxy.modeDirect")}</option>
            </select>
          </label>
          {mode === "system" && (
            <p className="text-xs text-muted-foreground">
              {t("settings.proxy.systemHint")}
            </p>
          )}
          {mode === "manual" && (
            <fieldset disabled={busy} className="min-w-0 space-y-4">
              {manualLoadError && (
                <p role="alert" className="text-sm text-destructive">
                  {t("settings.proxy.manualLoadError")}
                </p>
              )}
              <div className="grid gap-3 sm:grid-cols-[1fr_120px]">
                <label className="space-y-1.5">
                  <span className="text-sm font-medium">
                    {t("settings.proxy.host")}
                  </span>
                  <Input
                    value={host}
                    onChange={(event) => {
                      setHost(event.target.value);
                      setUnusedAddressPort(null);
                    }}
                    onBlur={() => {
                      if (address !== null) applyAddress(address);
                    }}
                    onPaste={(event) => {
                      const input = event.currentTarget;
                      const text = event.clipboardData.getData("text");
                      const start = input.selectionStart ?? input.value.length;
                      const end = input.selectionEnd ?? start;
                      const next =
                        input.value.slice(0, start) +
                        text +
                        input.value.slice(end);
                      const pasted = splitProxyAddress(next);
                      if (pasted === null || pasted.host === next) return;
                      event.preventDefault();
                      setUnusedAddressPort(null);
                      applyAddress(pasted);
                    }}
                    placeholder={t("settings.proxy.hostPlaceholder")}
                    aria-label={t("settings.proxy.host")}
                    aria-describedby={addressHintId}
                    aria-invalid={invalidAddress}
                    autoComplete="off"
                  />
                </label>
                <div className="space-y-1.5">
                  <label className="block space-y-1.5">
                    <span className="text-sm font-medium">
                      {t("settings.proxy.port")}
                    </span>
                    <Input
                      value={port}
                      onChange={(event) => {
                        setPort(event.target.value);
                        setUnusedAddressPort(null);
                      }}
                      placeholder={t("settings.proxy.portPlaceholder")}
                      aria-label={t("settings.proxy.port")}
                      aria-invalid={invalidPort}
                      aria-describedby={
                        invalidPort || unusedAddressPort !== null
                          ? portHintId
                          : undefined
                      }
                      inputMode="numeric"
                    />
                  </label>
                  {(invalidPort || unusedAddressPort !== null) && (
                    <p
                      id={portHintId}
                      className={
                        invalidPort
                          ? "text-xs text-destructive"
                          : "text-xs text-muted-foreground"
                      }
                    >
                      {invalidPort
                        ? t("settings.proxy.invalidPort")
                        : t("settings.proxy.keptPort", {
                            port: unusedAddressPort,
                          })}
                    </p>
                  )}
                </div>
              </div>
              <p
                id={addressHintId}
                className={
                  invalidAddress
                    ? "text-xs text-destructive"
                    : "text-xs text-muted-foreground"
                }
              >
                {t(
                  invalidAddress
                    ? "settings.proxy.invalidAddress"
                    : "settings.proxy.addressHint",
                )}
              </p>
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
            </fieldset>
          )}
          <div className="flex flex-wrap items-center gap-3">
            <Button
              variant="outline"
              disabled={busy || invalid}
              onClick={() => void save()}
            >
              {controller.isSaving
                ? t("settings.proxy.saving")
                : t("settings.proxy.save")}
            </Button>
            <Button
              variant="outline"
              disabled={busy}
              onClick={() => void clear()}
            >
              {t("settings.proxy.clear")}
            </Button>
            <Button
              variant="outline"
              disabled={checkBlocked || invalid}
              onClick={() => {
                setCheckUrl(controller.checkUrl);
                setCheckOpen(true);
              }}
            >
              {t("settings.proxy.check")}
            </Button>
            {controller.checkUrlError !== null && (
              <>
                <span role="alert" className="text-xs text-destructive">
                  {t("settings.proxy.checkUrlLoadError")}
                </span>
                <Button
                  variant="outline"
                  onClick={() => void controller.retry()}
                >
                  {t("common.retry")}
                </Button>
              </>
            )}
            {controller.loadError !== null && (
              <>
                <span role="alert" className="text-xs text-destructive">
                  {t("settings.proxy.loadError")}
                </span>
                <Button
                  variant="outline"
                  onClick={() => void controller.retry()}
                >
                  {t("common.retry")}
                </Button>
                {!replacingUnreadable && (
                  <Button variant="outline" onClick={onReplaceUnreadable}>
                    {t("settings.proxy.replaceUnreadable")}
                  </Button>
                )}
              </>
            )}
            {controller.updateError !== null && (
              <span role="alert" className="text-xs text-destructive">
                {t("settings.proxy.updateError")}
              </span>
            )}
          </div>
        </div>
      </div>
      {checkOpen && !invalid && (
        <CheckProxyDialog
          mode={mode}
          usingDraft={usingDraft}
          checking={controller.isChecking}
          blocked={checkBlocked}
          readError={
            controller.loadError !== null
              ? t("settings.proxy.loadError")
              : controller.checkUrlError !== null
                ? t("settings.proxy.checkUrlLoadError")
                : null
          }
          onRetry={() => void controller.retry()}
          url={checkUrl}
          onUrlChange={setCheckUrl}
          onOpenChange={(open) => {
            setCheckOpen(open);
            if (!open) setCheckUrl(controller.checkUrl);
          }}
          onSaveUrl={controller.saveCheckUrl}
          onCheck={(url) => controller.check(url, mode, parsed)}
        />
      )}
    </section>
  );
}

interface CheckProxyDialogProps {
  mode: ProxyMode;
  usingDraft: boolean;
  checking: boolean;
  blocked: boolean;
  readError: string | null;
  onRetry: () => void;
  url: string;
  onUrlChange: (url: string) => void;
  onOpenChange: (open: boolean) => void;
  onCheck: (url: string) => Promise<CheckProxySettingsResponse>;
  onSaveUrl: (url: string) => Promise<unknown>;
}

/** Prompts for a URL and reports whether the current form proxy can reach it. */
function CheckProxyDialog({
  mode,
  usingDraft,
  checking,
  blocked,
  readError,
  onRetry,
  url,
  onUrlChange,
  onOpenChange,
  onCheck,
  onSaveUrl,
}: CheckProxyDialogProps) {
  const { t } = useTranslation();

  const verify = async () => {
    if (blocked || checking) return;
    const nextUrl = url.trim();
    if (nextUrl === "") {
      toast.error(t("settings.proxy.checkUrlRequired"));
      return;
    }
    try {
      await onSaveUrl(nextUrl);
    } catch (cause) {
      toast.error(t("settings.proxy.checkUrlSaveError"), {
        description: localizeContractError(cause, t),
      });
      return;
    }
    try {
      const result = await onCheck(nextUrl);
      if (result.outcome === "reachable") {
        if (result.route === "direct" && mode === "manual") {
          toast.warning(t("settings.proxy.checkBypassedManual"), {
            description: t("settings.proxy.checkSuccessDetail", {
              status: result.status,
            }),
          });
          return;
        }
        const routeDetail =
          result.route === undefined
            ? t("settings.proxy.checkSuccessDetail", { status: result.status })
            : t("settings.proxy.checkRouteDetail", {
                status: result.status,
                route: t(
                  {
                    direct: "settings.proxy.modeDirect",
                    manual: "settings.proxy.modeManual",
                    system: "settings.proxy.routeSystem",
                    environment: "settings.proxy.routeEnvironment",
                  }[result.route],
                ),
              });
        toast.success(t("settings.proxy.checkSuccess"), {
          description: usingDraft
            ? routeDetail + " " + t("settings.proxy.checkDraft")
            : routeDetail,
        });
        onOpenChange(false);
        return;
      }
      if (result.outcome === "http_error") {
        toast.error(t("settings.proxy.checkHttpError"), {
          description: t("settings.proxy.checkSuccessDetail", {
            status: result.status,
          }),
        });
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
            disabled={checking}
            onChange={(event) => onUrlChange(event.target.value)}
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
        {usingDraft && (
          <p className="text-sm text-muted-foreground">
            {t("settings.proxy.checkDraft")}
          </p>
        )}
        {readError !== null && (
          <div className="space-y-2">
            <p role="alert">{readError}</p>
            <Button variant="outline" onClick={onRetry}>
              {t("common.retry")}
            </Button>
          </div>
        )}
        <DialogFooter>
          <Button
            variant="outline"
            disabled={checking}
            onClick={() => onOpenChange(false)}
          >
            {t("common.cancel")}
          </Button>
          <Button
            disabled={blocked || checking || url.trim() === ""}
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
