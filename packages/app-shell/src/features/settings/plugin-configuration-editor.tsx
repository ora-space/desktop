import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type {
  PluginConfigurationDetails,
  PluginSettingDetails,
  PluginSettingValue,
} from "@ora/contracts";
import { RemoteContractError } from "@ora/contracts";
import {
  AlertDialog,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Badge,
  Breadcrumb,
  BreadcrumbItem,
  BreadcrumbLink,
  BreadcrumbList,
  BreadcrumbPage,
  BreadcrumbSeparator,
  Button,
  Input,
} from "@ora/ui";
import { IconLoader2 } from "@tabler/icons-react";
import { localizeContractError } from "../../i18n/contract-error";
import { useContractErrorToast } from "../../i18n/use-contract-error-toast";
import { usePluginConfiguration } from "../../state/hooks/use-plugin-configuration";

type Draft = { override: boolean; value: string | boolean | null };
type Drafts = Record<string, Draft>;

/** Lets the Settings container protect navigation without owning editor draft state. */
export interface PluginConfigurationNavigationGuard {
  isDirty: () => boolean;
  save: () => Promise<boolean>;
}

/** Renders one host-owned third-level editor from the plugin's immutable declaration. */
export function PluginConfigurationEditor({
  pluginId,
  displayName,
  onBack,
  onNavigationGuardChange,
}: {
  pluginId: string;
  displayName: string;
  onBack: () => void;
  onNavigationGuardChange?: (
    guard: PluginConfigurationNavigationGuard | null,
  ) => void;
}) {
  const { t } = useTranslation();
  const configuration = usePluginConfiguration(pluginId);
  const [savedRevision, setSavedRevision] = useState<bigint | null>(null);

  if (configuration.query.isPending)
    return <IconLoader2 className="animate-spin text-muted-foreground" />;
  if (configuration.query.isError)
    return (
      <div className="space-y-4">
        <Button variant="ghost" onClick={onBack}>
          {t("settings.plugins.configuration.back")}
        </Button>
        <p className="text-sm text-destructive">
          {localizeContractError(configuration.query.error, t)}
        </p>
      </div>
    );

  const details = configuration.query.data;
  if (details.summary.state === "unavailable")
    return (
      <ConfigurationUnavailable
        displayName={displayName}
        pending={configuration.reset.isPending}
        onBack={onBack}
        onRecover={() =>
          configuration.reset.mutateAsync({
            declarationFingerprint: details.declarationFingerprint,
            mode: "recover_corrupt",
          })
        }
      />
    );

  return (
    <LoadedConfigurationEditor
      details={details}
      displayName={displayName}
      saved={savedRevision === details.revision}
      saving={configuration.save.isPending}
      resetting={configuration.reset.isPending}
      onBack={onBack}
      onNavigationGuardChange={onNavigationGuardChange}
      onSave={async (values, preserveSettingIds) => {
        const response = await configuration.save.mutateAsync({
          expectedRevision: details.revision,
          declarationFingerprint: details.declarationFingerprint,
          values,
          preserveSettingIds,
        });
        setSavedRevision(response.configuration.revision);
        return response.configuration;
      }}
      onReset={async () => {
        const response = await configuration.reset.mutateAsync({
          declarationFingerprint: details.declarationFingerprint,
          mode: "reset_all",
          expectedRevision: details.revision,
        });
        return response.configuration;
      }}
      onReload={() =>
        configuration.query.refetch().then((result) => {
          if (result.data === undefined)
            throw (
              result.error ??
              new Error("plugin configuration reload returned no data")
            );
          return result.data;
        })
      }
    />
  );
}

/** Keeps draft state local to one loaded revision so conflicts never discard unsaved input. */
function LoadedConfigurationEditor({
  details,
  displayName,
  saved,
  saving,
  resetting,
  onBack,
  onNavigationGuardChange,
  onSave,
  onReset,
  onReload,
}: {
  details: PluginConfigurationDetails;
  displayName: string;
  saved: boolean;
  saving: boolean;
  resetting: boolean;
  onBack: () => void;
  onNavigationGuardChange?: (
    guard: PluginConfigurationNavigationGuard | null,
  ) => void;
  onSave: (
    values: Record<string, PluginSettingValue>,
    preserveSettingIds: string[],
  ) => Promise<PluginConfigurationDetails>;
  onReset: () => Promise<PluginConfigurationDetails>;
  onReload: () => Promise<PluginConfigurationDetails>;
}) {
  const { t } = useTranslation();
  const showContractError = useContractErrorToast();
  const baseline = useMemo(() => draftsFrom(details), [details]);
  const [drafts, setDrafts] = useState<Drafts>(baseline);
  const [leaveOpen, setLeaveOpen] = useState(false);
  const [resetOpen, setResetOpen] = useState(false);
  const [fieldError, setFieldError] = useState<string | null>(null);
  const [reloadRequired, setReloadRequired] = useState(false);
  const inputBySettingId = useRef<
    Map<string, HTMLInputElement | HTMLSelectElement>
  >(new Map());
  const dirty = !sameDrafts(drafts, baseline);

  const focusField = (settingId: string) =>
    inputBySettingId.current.get(settingId)?.focus();

  const save = async () => {
    const values: Record<string, PluginSettingValue> = {};
    const preserveSettingIds: string[] = [];
    for (const field of details.settings) {
      const draft = drafts[field.declaration.id];
      if (field.redacted && !draft?.override) {
        preserveSettingIds.push(field.declaration.id);
        continue;
      }
      if (draft === undefined || !draft.override) continue;
      if (field.declaration.type === "number") {
        const text = String(draft.value ?? "");
        const value = Number(text);
        if (text.trim() === "" || !Number.isFinite(value)) {
          setFieldError(field.declaration.id);
          focusField(field.declaration.id);
          return false;
        }
        values[field.declaration.id] = value;
      } else if (field.declaration.type === "boolean") {
        if (typeof draft.value !== "boolean") {
          setFieldError(field.declaration.id);
          focusField(field.declaration.id);
          return false;
        }
        values[field.declaration.id] = draft.value;
      } else {
        values[field.declaration.id] = String(draft.value ?? "");
      }
    }
    setFieldError(null);
    try {
      const saved = await onSave(values, preserveSettingIds);
      setDrafts(draftsFrom(saved));
      return true;
    } catch (error) {
      if (
        error instanceof RemoteContractError &&
        error.payload.code === "plugin_configuration_validation"
      ) {
        const first = error.payload.params.fieldErrors[0]?.settingId;
        if (first !== undefined) {
          setFieldError(first);
          focusField(first);
        }
      }
      if (
        error instanceof RemoteContractError &&
        (error.payload.code === "configuration_revision_conflict" ||
          error.payload.code === "plugin_configuration_declaration_changed")
      )
        setReloadRequired(true);
      showContractError(error, t("settings.plugins.configuration.saveFailed"));
      return false;
    }
  };
  const dirtyRef = useRef(dirty);
  const saveRef = useRef(save);

  useEffect(() => {
    dirtyRef.current = dirty;
    saveRef.current = save;
  });

  useEffect(() => {
    const guard: PluginConfigurationNavigationGuard = {
      isDirty: () => dirtyRef.current,
      save: () => saveRef.current(),
    };
    onNavigationGuardChange?.(guard);
    return () => onNavigationGuardChange?.(null);
  }, [onNavigationGuardChange]);

  return (
    <div className="space-y-5">
      <ConfigurationBreadcrumb
        displayName={displayName}
        onBack={() => {
          if (dirty) setLeaveOpen(true);
          else onBack();
        }}
      />
      <header>
        <h2 className="text-lg font-semibold">{displayName}</h2>
        <p className="mt-1 text-sm text-muted-foreground">
          {t("settings.plugins.configuration.description")}
        </p>
      </header>

      <div className="space-y-5">
        {reloadRequired && (
          <div className="flex items-center justify-between gap-3 rounded-lg border border-destructive/40 p-3 text-sm text-destructive">
            <span>{t("settings.plugins.configuration.reloadRequired")}</span>
            <Button
              variant="outline"
              size="sm"
              onClick={() =>
                void onReload()
                  .then(() => setReloadRequired(false))
                  .catch((error: unknown) => showContractError(error))
              }
            >
              {t("settings.plugins.configuration.reload")}
            </Button>
          </div>
        )}
        {details.settings.map((field) => {
          const draft = drafts[field.declaration.id] ?? draftFrom(field);
          const update = (next: Draft) => {
            setFieldError((current) =>
              current === field.declaration.id ? null : current,
            );
            setDrafts((current) => ({
              ...current,
              [field.declaration.id]: next,
            }));
          };
          return (
            <div key={field.declaration.id} className="space-y-1.5">
              <label htmlFor={fieldId(field)} className="text-sm font-medium">
                {field.declaration.title}
                {field.declaration.required && (
                  <span className="ml-1 text-destructive">*</span>
                )}
              </label>
              <p className="text-xs text-muted-foreground">
                {field.declaration.description}
              </p>
              {field.declaration.type === "boolean" ? (
                <select
                  id={fieldId(field)}
                  ref={(element) => {
                    if (element === null)
                      inputBySettingId.current.delete(field.declaration.id);
                    else
                      inputBySettingId.current.set(
                        field.declaration.id,
                        element,
                      );
                  }}
                  aria-label={field.declaration.title}
                  className="h-8 w-full rounded-lg border border-input bg-background px-2 text-sm"
                  value={draft.override ? String(draft.value) : "unset"}
                  onChange={(event) => {
                    const value = event.target.value;
                    update(
                      value === "unset"
                        ? draftFrom(field)
                        : { override: true, value: value === "true" },
                    );
                  }}
                >
                  <option value="unset">
                    {field.declaration.default === null
                      ? t("settings.plugins.configuration.notSet")
                      : t("settings.plugins.configuration.useDefault")}
                  </option>
                  <option value="true">
                    {t("settings.plugins.configuration.on")}
                  </option>
                  <option value="false">
                    {t("settings.plugins.configuration.off")}
                  </option>
                </select>
              ) : (
                <Input
                  id={fieldId(field)}
                  ref={(element) => {
                    if (element === null)
                      inputBySettingId.current.delete(field.declaration.id);
                    else
                      inputBySettingId.current.set(
                        field.declaration.id,
                        element,
                      );
                  }}
                  aria-label={field.declaration.title}
                  type={field.redacted ? "password" : undefined}
                  placeholder={
                    field.redacted && field.source === "stored"
                      ? t("settings.plugins.configuration.configuredSecret")
                      : undefined
                  }
                  inputMode={
                    field.declaration.type === "number" ? "decimal" : undefined
                  }
                  value={String(draft.value ?? "")}
                  aria-invalid={fieldError === field.declaration.id}
                  onChange={(event) =>
                    update({ override: true, value: event.target.value })
                  }
                />
              )}
              <div className="flex items-center gap-2">
                {!draft.override && field.source === "default" && (
                  <Badge variant="secondary">
                    {t("settings.plugins.configuration.default")}
                  </Badge>
                )}
                {draft.override && (
                  <Button
                    variant="ghost"
                    size="sm"
                    onClick={() => update(resetDraftFrom(field))}
                  >
                    {t("settings.plugins.configuration.resetField")}
                  </Button>
                )}
              </div>
              {field.valueErrorCode !== null && (
                <p className="text-xs text-destructive">
                  {t("settings.plugins.configuration.storedValueInvalid")}
                </p>
              )}
            </div>
          );
        })}
      </div>

      <div className="flex items-center gap-2 border-t pt-4">
        <Button disabled={!dirty || saving} onClick={() => void save()}>
          {saving
            ? t("common.saving")
            : t("settings.plugins.configuration.save")}
        </Button>
        <Button
          variant="outline"
          disabled={resetting}
          onClick={() => setResetOpen(true)}
        >
          {t("settings.plugins.configuration.resetAll")}
        </Button>
        {saved && (
          <span className="text-sm text-muted-foreground">
            {t("settings.plugins.configuration.saved")}
          </span>
        )}
      </div>

      <AlertDialog open={leaveOpen} onOpenChange={setLeaveOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.plugins.configuration.unsavedTitle")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.plugins.configuration.unsavedDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <Button variant="outline" onClick={onBack}>
              {t("settings.plugins.configuration.discard")}
            </Button>
            <Button
              disabled={saving}
              onClick={() => void save().then((ok) => ok && onBack())}
            >
              {t("settings.plugins.configuration.save")}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>

      <AlertDialog open={resetOpen} onOpenChange={setResetOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.plugins.configuration.resetTitle")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.plugins.configuration.resetDescription")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <Button
              disabled={resetting}
              onClick={() =>
                void onReset()
                  .then((configuration) => {
                    setDrafts(draftsFrom(configuration));
                    setResetOpen(false);
                  })
                  .catch((error: unknown) => showContractError(error))
              }
            >
              {t("settings.plugins.configuration.resetAll")}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** Displays damaged storage separately from ordinary incomplete configuration. */
function ConfigurationUnavailable({
  displayName,
  pending,
  onBack,
  onRecover,
}: {
  displayName: string;
  pending: boolean;
  onBack: () => void;
  onRecover: () => Promise<unknown>;
}) {
  const { t } = useTranslation();
  const showContractError = useContractErrorToast();
  const [open, setOpen] = useState(false);
  return (
    <div className="space-y-5">
      <ConfigurationBreadcrumb displayName={displayName} onBack={onBack} />
      <h2 className="text-lg font-semibold">{displayName}</h2>
      <p className="text-sm text-destructive">
        {t("settings.plugins.configuration.unavailable")}
      </p>
      <Button disabled={pending} onClick={() => setOpen(true)}>
        {t("settings.plugins.configuration.recover")}
      </Button>
      <AlertDialog open={open} onOpenChange={setOpen}>
        <AlertDialogContent>
          <AlertDialogHeader>
            <AlertDialogTitle>
              {t("settings.plugins.configuration.recoverTitle")}
            </AlertDialogTitle>
            <AlertDialogDescription>
              {t("settings.plugins.configuration.unavailable")}
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel>
            <Button
              disabled={pending}
              onClick={() =>
                void onRecover()
                  .then(() => setOpen(false))
                  .catch((error: unknown) => showContractError(error))
              }
            >
              {t("settings.plugins.configuration.recover")}
            </Button>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** Keeps third-level navigation inside the Settings Dialog. */
function ConfigurationBreadcrumb({
  displayName,
  onBack,
}: {
  displayName: string;
  onBack: () => void;
}) {
  const { t } = useTranslation();
  return (
    <Breadcrumb>
      <BreadcrumbList>
        <BreadcrumbItem>
          <BreadcrumbLink render={<button type="button" onClick={onBack} />}>
            {t("settings.plugins.manageInstalled")}
          </BreadcrumbLink>
        </BreadcrumbItem>
        <BreadcrumbSeparator />
        <BreadcrumbItem>
          <BreadcrumbPage>{displayName}</BreadcrumbPage>
        </BreadcrumbItem>
      </BreadcrumbList>
    </Breadcrumb>
  );
}

function draftFrom(field: PluginSettingDetails): Draft {
  if (field.redacted) return { override: false, value: null };
  if (field.storedValue !== null)
    return { override: true, value: displayValue(field.storedValue) };
  return { override: false, value: displayValue(field.effectiveValue) };
}

/** Removes an explicit override while restoring the declaration default or absent state. */
function resetDraftFrom(field: PluginSettingDetails): Draft {
  return {
    override: false,
    value: displayValue(field.declaration.default),
  };
}

function draftsFrom(details: PluginConfigurationDetails): Drafts {
  return Object.fromEntries(
    details.settings.map((field) => [field.declaration.id, draftFrom(field)]),
  );
}

function displayValue(
  value: PluginSettingValue | null,
): string | boolean | null {
  return typeof value === "number" ? String(value) : value;
}

function sameDrafts(left: Drafts, right: Drafts): boolean {
  const keys = Object.keys(left);
  return (
    keys.length === Object.keys(right).length &&
    keys.every(
      (key) =>
        left[key]?.override === right[key]?.override &&
        left[key]?.value === right[key]?.value,
    )
  );
}

function fieldId(field: PluginSettingDetails): string {
  return `plugin-setting-${field.declaration.id}`;
}
