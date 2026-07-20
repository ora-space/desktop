import { useState } from "react";
import { useTranslation } from "react-i18next";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
  Badge,
  Button,
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
  ScrollArea,
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  Switch,
  cn,
} from "@ora/ui";
import {
  IconAdjustments,
  IconAtom,
  IconCheck,
  IconDatabase,
  IconDeviceDesktop,
  IconLanguage,
  IconLock,
  IconMoon,
  IconPlugConnected,
  IconRobot,
  IconShieldCheck,
  IconSun,
  IconTrash,
} from "@tabler/icons-react";
import type { Locale } from "../../i18n/i18n";
import { AtomsSettings } from "./atoms-settings";
import { useUiStore } from "../../state/stores/ui-store";
import { useSettingsStore, type SettingsPreferences } from "../../state/stores/settings-store";
import { useChatStore } from "../../chat-store-context";
import { useStore } from "zustand";
import type {
  ApprovalPolicy,
  InterfaceDensity,
  ModelProvider,
  ThemeMode,
} from "../../state/stores/settings-store";

type SettingsCategory = "appearance" | "atoms" | "models" | "permissions" | "privacy";

/** Presents shared Ora preferences in a dense IDE-style settings surface. */
export function SettingsDialog() {
  const { t } = useTranslation();
  const open = useUiStore((s) => s.settingsOpen);
  const setOpen = useUiStore((s) => s.setSettingsOpen);
  const settings = useSettingsStore((s) => s.settings);
  const updateSettings = useSettingsStore((s) => s.updateSettings);
  const chatStore = useChatStore();
  const clearConversations = useStore(chatStore, (state) => state.clearAll);
  const [category, setCategory] = useState<SettingsCategory>("appearance");

  const categories: Array<{ id: SettingsCategory; icon: typeof IconAdjustments; label: string }> = [
    { id: "appearance", icon: IconAdjustments, label: t("settings.nav.appearance") },
    { id: "atoms", icon: IconAtom, label: t("settings.nav.atoms") },
    { id: "models", icon: IconRobot, label: t("settings.nav.models") },
    { id: "permissions", icon: IconShieldCheck, label: t("settings.nav.permissions") },
    { id: "privacy", icon: IconDatabase, label: t("settings.nav.privacy") },
  ];

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent
        showCloseButton
        className="h-[min(720px,calc(100dvh-2rem))] w-[min(1040px,calc(100vw-2rem))] max-w-none gap-0 overflow-hidden p-0 sm:max-w-none"
      >
        <DialogHeader className="sr-only">
          <DialogTitle>{t("common.settings")}</DialogTitle>
          <DialogDescription>{t("settings.description")}</DialogDescription>
        </DialogHeader>
        <div className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] sm:grid-cols-[210px_minmax(0,1fr)] sm:grid-rows-1">
          <aside className="border-b border-border bg-muted/35 p-3 sm:border-b-0 sm:border-r">
            <div className="hidden h-11 items-center gap-2 px-2 sm:flex">
              <div className="flex size-7 items-center justify-center rounded-md bg-foreground text-background"><IconAdjustments className="size-4" /></div>
              <span className="text-sm font-semibold">{t("common.settings")}</span>
            </div>
            <nav className="flex gap-1 overflow-x-auto sm:mt-3 sm:flex-col" aria-label={t("common.settings")}>
              {categories.map((item) => {
                const Icon = item.icon;
                return (
                  <button
                    key={item.id}
                    type="button"
                    onClick={() => setCategory(item.id)}
                    className={cn(
                      "flex h-9 shrink-0 items-center gap-2 rounded-md px-2.5 text-left text-sm font-medium outline-none transition-colors focus-visible:ring-2 focus-visible:ring-ring sm:w-full",
                      category === item.id ? "bg-background text-foreground shadow-sm ring-1 ring-border" : "text-muted-foreground hover:bg-background/70 hover:text-foreground",
                    )}
                  >
                    <Icon className="size-4" />
                    <span>{item.label}</span>
                  </button>
                );
              })}
            </nav>
            <p className="mt-auto hidden px-2 pb-1 pt-6 text-[10px] leading-4 text-muted-foreground sm:block">{t("settings.productName")}<br />{t("settings.prototypeLabel")}</p>
          </aside>

          <ScrollArea className="min-h-0">
            <div className="mx-auto w-full max-w-3xl p-5 pb-12 sm:p-8 sm:pb-12">
              {category === "appearance" && <AppearanceSettings settings={settings} onUpdate={updateSettings} />}
              {category === "atoms" && <AtomsSettings />}
              {category === "models" && <ModelSettings settings={settings} onUpdate={updateSettings} />}
              {category === "permissions" && <PermissionSettings settings={settings} onUpdate={updateSettings} />}
              {category === "privacy" && <PrivacySettings settings={settings} onUpdate={updateSettings} onClearHistory={clearConversations} />}
            </div>
          </ScrollArea>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** Applies visual and locale preferences immediately so users can evaluate the result in context. */
function AppearanceSettings({ settings, onUpdate }: { settings: SettingsPreferences; onUpdate: (patch: Partial<SettingsPreferences>) => void }) {
  const { i18n, t } = useTranslation();
  const locale: Locale = i18n.resolvedLanguage === "en-US" ? "en-US" : "zh-CN";
  const themes: Array<{ value: ThemeMode; icon: typeof IconSun; label: string }> = [
    { value: "system", icon: IconDeviceDesktop, label: t("settings.appearance.system") },
    { value: "light", icon: IconSun, label: t("settings.appearance.light") },
    { value: "dark", icon: IconMoon, label: t("settings.appearance.dark") },
  ];

  return (
    <div className="space-y-7">
      <SettingsHeading title={t("settings.appearance.title")} description={t("settings.appearance.description")} />
      <SettingsGroup title={t("settings.appearance.theme")} description={t("settings.appearance.themeDescription")}>
        <div className="grid grid-cols-3 gap-2">
          {themes.map((theme) => {
            const Icon = theme.icon;
            const selected = settings.theme === theme.value;
            return (
              <button
                key={theme.value}
                type="button"
                aria-pressed={selected}
                onClick={() => onUpdate({ theme: theme.value })}
                className={cn("relative overflow-hidden rounded-md border p-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring", selected ? "border-foreground" : "border-border hover:border-foreground/40")}
              >
                <div className={cn("mb-2 h-14 rounded-sm border p-1.5", theme.value === "dark" ? "border-zinc-700 bg-zinc-950" : theme.value === "light" ? "bg-white" : "bg-gradient-to-r from-white from-50% to-zinc-950 to-50%") }>
                  <div className={cn("h-1.5 w-8 rounded-full", theme.value === "dark" ? "bg-zinc-600" : "bg-zinc-300")} />
                  <div className={cn("mt-2 h-5 rounded-sm border", theme.value === "dark" ? "border-zinc-700 bg-zinc-900" : "border-zinc-200 bg-zinc-50")} />
                </div>
                <span className="flex items-center gap-1.5 text-xs font-medium"><Icon className="size-3.5" />{theme.label}</span>
                {selected && <IconCheck className="absolute right-2 top-2 size-3.5" />}
              </button>
            );
          })}
        </div>
      </SettingsGroup>
      <SettingsRow icon={IconLanguage} title={t("settings.appearance.language")} description={t("settings.appearance.languageDescription")}>
        <Select value={locale} onValueChange={(value) => void i18n.changeLanguage(value as Locale)}>
          <SelectTrigger className="w-40"><span className="flex-1 text-left">{locale === "zh-CN" ? t("account.switchChinese") : t("account.switchEnglish")}</span></SelectTrigger>
          <SelectContent><SelectItem value="zh-CN">{t("account.switchChinese")}</SelectItem><SelectItem value="en-US">{t("account.switchEnglish")}</SelectItem></SelectContent>
        </Select>
      </SettingsRow>
      <SettingsRow icon={IconAdjustments} title={t("settings.appearance.density")} description={t("settings.appearance.densityDescription")}>
        <Select value={settings.density} onValueChange={(value) => onUpdate({ density: value as InterfaceDensity })}>
          <SelectTrigger className="w-40"><span className="flex-1 text-left">{settings.density === "comfortable" ? t("settings.appearance.comfortable") : t("settings.appearance.compact")}</span></SelectTrigger>
          <SelectContent><SelectItem value="comfortable">{t("settings.appearance.comfortable")}</SelectItem><SelectItem value="compact">{t("settings.appearance.compact")}</SelectItem></SelectContent>
        </Select>
      </SettingsRow>
    </div>
  );
}

/** Models provider choices without collecting secrets until a secure credential contract exists. */
function ModelSettings({ settings, onUpdate }: { settings: SettingsPreferences; onUpdate: (patch: Partial<SettingsPreferences>) => void }) {
  const { t } = useTranslation();
  const models: Record<ModelProvider, string[]> = {
    openai: ["gpt-5.1-codex", "gpt-5.1", "gpt-4.1"],
    anthropic: ["claude-sonnet-4.5", "claude-opus-4.1"],
    local: ["qwen3-coder", "deepseek-r1"],
  };
  const changeProvider = (provider: ModelProvider) => onUpdate({ provider, model: models[provider][0] });

  return (
    <div className="space-y-7">
      <SettingsHeading title={t("settings.models.title")} description={t("settings.models.description")} />
      <SettingsRow icon={IconPlugConnected} title={t("settings.models.provider")} description={t("settings.models.providerDescription")}>
        <Select value={settings.provider} onValueChange={(value) => changeProvider(value as ModelProvider)}>
          <SelectTrigger className="w-44"><span className="flex-1 text-left">{settings.provider === "local" ? t("settings.models.local") : settings.provider === "openai" ? "OpenAI" : "Anthropic"}</span></SelectTrigger>
          <SelectContent><SelectItem value="openai">OpenAI</SelectItem><SelectItem value="anthropic">Anthropic</SelectItem><SelectItem value="local">{t("settings.models.local")}</SelectItem></SelectContent>
        </Select>
      </SettingsRow>
      <SettingsRow icon={IconRobot} title={t("settings.models.defaultModel")} description={t("settings.models.defaultModelDescription")}>
        <Select value={settings.model} onValueChange={(model) => onUpdate({ model: model as string })}>
          <SelectTrigger className="w-52"><span className="flex-1 text-left">{settings.model}</span></SelectTrigger>
          <SelectContent>{models[settings.provider].map((model) => <SelectItem key={model} value={model}>{model}</SelectItem>)}</SelectContent>
        </Select>
      </SettingsRow>
      <div className="flex items-center gap-3 border-y border-border py-4">
        <span className="relative flex size-2"><span className="absolute inline-flex size-full animate-ping rounded-full bg-emerald-500 opacity-40" /><span className="relative inline-flex size-2 rounded-full bg-emerald-500" /></span>
        <div className="min-w-0 flex-1"><p className="text-sm font-medium">{t("settings.models.connected")}</p><p className="text-xs text-muted-foreground">{t("settings.models.prototypeConnection")}</p></div>
        <Badge variant="outline">{t("settings.models.ready")}</Badge>
      </div>
    </div>
  );
}

/** Captures the approval and capability controls expected before running agent commands. */
function PermissionSettings({ settings, onUpdate }: { settings: SettingsPreferences; onUpdate: (patch: Partial<SettingsPreferences>) => void }) {
  const { t } = useTranslation();
  return (
    <div className="space-y-7">
      <SettingsHeading title={t("settings.permissions.title")} description={t("settings.permissions.description")} />
      <SettingsRow icon={IconShieldCheck} title={t("settings.permissions.approval")} description={t("settings.permissions.approvalDescription")}>
        <Select value={settings.approvalPolicy} onValueChange={(value) => onUpdate({ approvalPolicy: value as ApprovalPolicy })}>
          <SelectTrigger className="w-48"><span className="flex-1 text-left">{settings.approvalPolicy === "always" ? t("settings.permissions.always") : settings.approvalPolicy === "risky" ? t("settings.permissions.risky") : t("settings.permissions.trusted")}</span></SelectTrigger>
          <SelectContent><SelectItem value="always">{t("settings.permissions.always")}</SelectItem><SelectItem value="risky">{t("settings.permissions.risky")}</SelectItem><SelectItem value="trusted">{t("settings.permissions.trusted")}</SelectItem></SelectContent>
        </Select>
      </SettingsRow>
      <div className="divide-y divide-border border-y border-border">
        <SwitchRow title={t("settings.permissions.terminal")} description={t("settings.permissions.terminalDescription")} checked={settings.terminalAccess} onCheckedChange={(terminalAccess) => onUpdate({ terminalAccess })} />
        <SwitchRow title={t("settings.permissions.files")} description={t("settings.permissions.filesDescription")} checked={settings.fileWriteAccess} onCheckedChange={(fileWriteAccess) => onUpdate({ fileWriteAccess })} />
        <SwitchRow title={t("settings.permissions.network")} description={t("settings.permissions.networkDescription")} checked={settings.networkAccess} onCheckedChange={(networkAccess) => onUpdate({ networkAccess })} />
      </div>
      <SettingsRow icon={IconLock} title={t("settings.permissions.timeout")} description={t("settings.permissions.timeoutDescription")}>
        <Select value={settings.commandTimeout} onValueChange={(commandTimeout) => onUpdate({ commandTimeout: commandTimeout as string })}>
          <SelectTrigger className="w-36"><span className="flex-1 text-left">{settings.commandTimeout === "30" ? t("settings.permissions.timeoutSeconds", { count: 30 }) : settings.commandTimeout === "120" ? t("settings.permissions.timeoutMinutes", { count: 2 }) : settings.commandTimeout === "300" ? t("settings.permissions.timeoutMinutes", { count: 5 }) : t("settings.permissions.noTimeout")}</span></SelectTrigger>
          <SelectContent>
            <SelectItem value="30">{t("settings.permissions.timeoutSeconds", { count: 30 })}</SelectItem>
            <SelectItem value="120">{t("settings.permissions.timeoutMinutes", { count: 2 })}</SelectItem>
            <SelectItem value="300">{t("settings.permissions.timeoutMinutes", { count: 5 })}</SelectItem>
            <SelectItem value="0">{t("settings.permissions.noTimeout")}</SelectItem>
          </SelectContent>
        </Select>
      </SettingsRow>
    </div>
  );
}

/** Groups local retention and diagnostic controls, including a confirmed history reset. */
function PrivacySettings({ settings, onUpdate, onClearHistory }: { settings: SettingsPreferences; onUpdate: (patch: Partial<SettingsPreferences>) => void; onClearHistory: () => void }) {
  const { t } = useTranslation();
  const [confirmClear, setConfirmClear] = useState(false);
  return (
    <div className="space-y-7">
      <SettingsHeading title={t("settings.privacy.title")} description={t("settings.privacy.description")} />
      <div className="border-y border-border">
        <SwitchRow title={t("settings.privacy.diagnostics")} description={t("settings.privacy.diagnosticsDescription")} checked={settings.diagnostics} onCheckedChange={(diagnostics) => onUpdate({ diagnostics })} />
      </div>
      <div className="flex flex-col gap-3 border-b border-border pb-5 sm:flex-row sm:items-center">
        <div className="min-w-0 flex-1"><p className="text-sm font-medium">{t("settings.privacy.clearHistory")}</p><p className="mt-1 text-xs leading-5 text-muted-foreground">{t("settings.privacy.clearHistoryDescription")}</p></div>
        <Button variant="destructive" onClick={() => setConfirmClear(true)}><IconTrash />{t("settings.privacy.clear")}</Button>
      </div>
      <AlertDialog open={confirmClear} onOpenChange={setConfirmClear}>
        <AlertDialogContent>
          <AlertDialogHeader><AlertDialogTitle>{t("settings.privacy.clearTitle")}</AlertDialogTitle><AlertDialogDescription>{t("settings.privacy.clearConfirm")}</AlertDialogDescription></AlertDialogHeader>
          <AlertDialogFooter><AlertDialogCancel>{t("common.cancel")}</AlertDialogCancel><AlertDialogAction variant="destructive" onClick={() => { onClearHistory(); setConfirmClear(false); }}><IconTrash />{t("settings.privacy.clear")}</AlertDialogAction></AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

/** Gives every settings pane a consistent heading and readable measure. */
function SettingsHeading({ title, description }: { title: string; description: string }) {
  return <header><h2 className="text-lg font-semibold">{title}</h2><p className="mt-1 max-w-2xl text-sm leading-6 text-muted-foreground">{description}</p></header>;
}

/** Labels a grouped control without introducing nested decorative cards. */
function SettingsGroup({ title, description, children }: { title: string; description: string; children: React.ReactNode }) {
  return <section><div className="mb-3"><h3 className="text-sm font-medium">{title}</h3><p className="mt-1 text-xs leading-5 text-muted-foreground">{description}</p></div>{children}</section>;
}

/** Aligns a preference description with its compact trailing control. */
function SettingsRow({ icon: Icon, title, description, children }: { icon: typeof IconAdjustments; title: string; description: string; children: React.ReactNode }) {
  return <div className="flex flex-col gap-3 border-y border-border py-4 sm:flex-row sm:items-center"><Icon className="hidden size-4 shrink-0 text-muted-foreground sm:block" /><div className="min-w-0 flex-1"><p className="text-sm font-medium">{title}</p><p className="mt-1 text-xs leading-5 text-muted-foreground">{description}</p></div><div className="shrink-0">{children}</div></div>;
}

/** Uses a switch for one self-contained runtime capability. */
function SwitchRow({ title, description, checked, onCheckedChange }: { title: string; description: string; checked: boolean; onCheckedChange: (checked: boolean) => void }) {
  return <div className="flex items-center gap-4 py-4"><div className="min-w-0 flex-1"><p className="text-sm font-medium">{title}</p><p className="mt-1 text-xs leading-5 text-muted-foreground">{description}</p></div><Switch checked={checked} onCheckedChange={onCheckedChange} /></div>;
}
